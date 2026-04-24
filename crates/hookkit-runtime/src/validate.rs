//! Output validation — checks outputs before emission.

use hookkit_claude::output::OutputEnvelope as ClaudeEnvelope;
use hookkit_codex::output::OutputEnvelope as CodexEnvelope;
use hookkit_core::{Harness, HookEventKey, HookkitError};
use hookkit_gemini::output::OutputEnvelope as GeminiEnvelope;
use std::path::Path;

fn invalid(harness: Harness, event: &HookEventKey, message: impl Into<String>) -> HookkitError {
    HookkitError::InvalidOutputCombination {
        harness,
        event: event.clone(),
        message: message.into(),
    }
}

fn require_reason(
    harness: Harness,
    event: &HookEventKey,
    decision: Option<&str>,
    reason: Option<&str>,
) -> Result<(), HookkitError> {
    let Some(decision) = decision else {
        return Ok(());
    };
    if reason.is_some() || !decision_requires_reason(decision) {
        return Ok(());
    }
    Err(invalid(
        harness,
        event,
        format!("decision '{decision}' requires a reason"),
    ))
}

fn decision_requires_reason(decision: &str) -> bool {
    matches!(decision, "ask" | "block" | "deny" | "retry" | "stop")
}

fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
}

/// Validate a Claude output envelope.
pub fn validate_claude(event: &HookEventKey, envelope: &ClaudeEnvelope) -> Result<(), HookkitError> {
    // Block requires a reason
    require_reason(
        Harness::Claude,
        event,
        envelope.decision.as_deref(),
        envelope.reason.as_deref(),
    )?;

    // PreToolUse must use hookSpecificOutput.permissionDecision, not top-level decision.
    if matches!(event, HookEventKey::PreToolUse) {
        if envelope.decision.is_some() || envelope.reason.is_some() {
            return Err(invalid(
                Harness::Claude,
                event,
                "PreToolUse must use hookSpecificOutput.permissionDecision, not top-level decision/reason",
            ));
        }

        if let Some(hso) = &envelope.hook_specific_output {
            let Some(permission) = hso.get("permissionDecision") else {
                return Err(invalid(
                    Harness::Claude,
                    event,
                    "PreToolUse hookSpecificOutput must include permissionDecision",
                ));
            };
            let decision = permission.get("decision").and_then(|v| v.as_str());
            match decision {
                Some("allow") => {}
                Some("deny") | Some("ask") => {
                    if permission.get("reason").and_then(|v| v.as_str()).is_none() {
                        return Err(invalid(
                            Harness::Claude,
                            event,
                            "permissionDecision deny/ask requires a reason",
                        ));
                    }
                }
                _ => {
                    return Err(invalid(
                        Harness::Claude,
                        event,
                        "permissionDecision.decision must be one of: allow, deny, ask",
                    ));
                }
            }
        }
    }

    // Cannot have both top-level decision and hookSpecificOutput.permissionDecision
    if envelope.decision.is_some() {
        if let Some(hso) = &envelope.hook_specific_output {
            if hso.get("permissionDecision").is_some() {
                return Err(invalid(
                    Harness::Claude,
                    event,
                    "cannot set both top-level decision and hookSpecificOutput.permissionDecision",
                ));
            }
        }
    }

    if matches!(event, HookEventKey::Other(name) if name == "WorktreeCreate") {
        if let Some(path) = envelope
            .hook_specific_output
            .as_ref()
            .and_then(|hso| hso.get("worktreePath"))
            .and_then(|v| v.as_str())
        {
            if !is_absolute(path) {
                return Err(invalid(
                    Harness::Claude,
                    event,
                    "worktreePath must be an absolute path",
                ));
            }
        }
    }

    if matches!(event, HookEventKey::Other(name) if name == "FileChanged")
        || matches!(event, HookEventKey::PreToolUse | HookEventKey::PostToolUse)
    {
        if let Some(paths) = envelope
            .hook_specific_output
            .as_ref()
            .and_then(|hso| hso.get("watchPaths"))
            .and_then(|v| v.as_array())
        {
            for p in paths {
                let Some(path) = p.as_str() else {
                    return Err(invalid(
                        Harness::Claude,
                        event,
                        "watchPaths entries must be strings",
                    ));
                };
                if !is_absolute(path) {
                    return Err(invalid(
                        Harness::Claude,
                        event,
                        "watchPaths entries must be absolute paths",
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Validate a Codex output envelope.
pub fn validate_codex(event: &HookEventKey, envelope: &CodexEnvelope) -> Result<(), HookkitError> {
    // Deny requires a reason
    require_reason(
        Harness::Codex,
        event,
        envelope.decision.as_deref(),
        envelope.reason.as_deref(),
    )?;

    if matches!(event, HookEventKey::Stop) {
        let is_empty = envelope.decision.is_none()
            && envelope.reason.is_none()
            && envelope.continue_session.is_none()
            && envelope.stop_reason.is_none()
            && envelope.suppress_output.is_none();
        if is_empty {
            return Err(invalid(
                Harness::Codex,
                event,
                "Stop output must not be an empty JSON object",
            ));
        }
    }

    Ok(())
}

/// Validate a Gemini output envelope.
pub fn validate_gemini(event: &HookEventKey, envelope: &GeminiEnvelope) -> Result<(), HookkitError> {
    // Deny requires a reason
    require_reason(
        Harness::Gemini,
        event,
        envelope.decision.as_deref(),
        envelope.reason.as_deref(),
    )?;

    if matches!(event, HookEventKey::BeforeToolSelection)
        && (envelope.decision.is_some()
            || envelope.reason.is_some()
            || envelope.continue_session.is_some()
            || envelope.stop_reason.is_some())
    {
        return Err(invalid(
            Harness::Gemini,
            event,
            "BeforeToolSelection does not support top-level decision/continue/stop fields",
        ));
    }

    if matches!(event, HookEventKey::PreToolUse) {
        if let Some(tool_input) = envelope
            .hook_specific_output
            .as_ref()
            .and_then(|hso| hso.get("tool_input"))
        {
            if !tool_input.is_object() {
                return Err(invalid(
                    Harness::Gemini,
                    event,
                    "hookSpecificOutput.tool_input must be a JSON object",
                ));
            }
        }
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
        assert!(validate_claude(&HookEventKey::PostToolUse, &env).is_err());
    }

    #[test]
    fn claude_block_with_reason_ok() {
        let env = ClaudeEnvelope::block("test");
        assert!(validate_claude(&HookEventKey::PostToolUse, &env).is_ok());
    }

    #[test]
    fn claude_allow_without_reason_ok() {
        let env = ClaudeEnvelope::allow();
        assert!(validate_claude(&HookEventKey::PromptSubmit, &env).is_ok());
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
        assert!(validate_claude(&HookEventKey::PreToolUse, &env).is_err());
    }

    #[test]
    fn claude_pretool_rejects_top_level_decision() {
        let env = ClaudeEnvelope::block("nope");
        assert!(validate_claude(&HookEventKey::PreToolUse, &env).is_err());
    }

    #[test]
    fn claude_worktree_path_requires_absolute() {
        let env = ClaudeEnvelope::worktree_path("relative/path");
        assert!(validate_claude(
            &HookEventKey::Other("WorktreeCreate".to_string()),
            &env
        )
        .is_err());
    }

    #[test]
    fn claude_watch_paths_require_absolute_entries() {
        let env = ClaudeEnvelope::watch_paths(vec!["/tmp/ok".to_string(), "relative".to_string()]);
        assert!(
            validate_claude(&HookEventKey::Other("FileChanged".to_string()), &env).is_err()
        );
    }

    #[test]
    fn codex_deny_without_reason_fails() {
        let env = CodexEnvelope {
            decision: Some("deny".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_codex(&HookEventKey::PreToolUse, &env).is_err());
    }

    #[test]
    fn codex_deny_with_reason_ok() {
        let env = CodexEnvelope::deny("test");
        assert!(validate_codex(&HookEventKey::PreToolUse, &env).is_ok());
    }

    #[test]
    fn codex_stop_empty_json_fails() {
        let env = CodexEnvelope::new();
        assert!(validate_codex(&HookEventKey::Stop, &env).is_err());
    }

    #[test]
    fn gemini_deny_without_reason_fails() {
        let env = GeminiEnvelope {
            decision: Some("deny".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_gemini(&HookEventKey::PreToolUse, &env).is_err());
    }

    #[test]
    fn gemini_retry_without_reason_fails() {
        let env = GeminiEnvelope {
            decision: Some("retry".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_gemini(&HookEventKey::Stop, &env).is_err());
    }

    #[test]
    fn gemini_retry_with_reason_ok() {
        let env = GeminiEnvelope::retry("test");
        assert!(validate_gemini(&HookEventKey::Stop, &env).is_ok());
    }

    #[test]
    fn gemini_before_tool_selection_rejects_decision() {
        let env = GeminiEnvelope::deny("not allowed");
        assert!(validate_gemini(&HookEventKey::BeforeToolSelection, &env).is_err());
    }

    #[test]
    fn gemini_before_tool_selection_allows_filter_tools() {
        let env = GeminiEnvelope::filter_tools(vec!["shell".to_string()]);
        assert!(validate_gemini(&HookEventKey::BeforeToolSelection, &env).is_ok());
    }

    #[test]
    fn gemini_tool_rewrite_must_be_object() {
        let env = GeminiEnvelope::rewrite_tool_input(serde_json::json!(["bad"]));
        assert!(validate_gemini(&HookEventKey::PreToolUse, &env).is_err());
    }
}
