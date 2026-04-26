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

fn ensure_block_only(
    harness: Harness,
    event: &HookEventKey,
    decision: Option<&str>,
    reason: Option<&str>,
) -> Result<(), HookkitError> {
    match decision {
        Some("block") => require_reason(harness, event, decision, reason),
        Some(other) => Err(invalid(
            harness,
            event,
            format!("decision must be 'block' when present, got '{other}'"),
        )),
        None if reason.is_some() => Err(invalid(
            harness,
            event,
            "reason requires decision: 'block'",
        )),
        None => Ok(()),
    }
}

fn reject_top_level_decision(
    harness: Harness,
    event: &HookEventKey,
    envelope: &impl std::borrow::Borrow<ClaudeEnvelope>,
) -> Result<(), HookkitError> {
    let envelope = envelope.borrow();
    if envelope.decision.is_some() || envelope.reason.is_some() {
        return Err(invalid(
            harness,
            event,
            "top-level decision/reason is not supported for this event",
        ));
    }
    Ok(())
}

fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
}

/// Validate a Claude output envelope.
pub fn validate_claude(
    event: &HookEventKey,
    envelope: &ClaudeEnvelope,
) -> Result<(), HookkitError> {
    match event {
        HookEventKey::PromptSubmit
        | HookEventKey::PromptExpansion
        | HookEventKey::PostToolUse
        | HookEventKey::PostToolUseFailure
        | HookEventKey::PostToolBatch
        | HookEventKey::Stop
        | HookEventKey::SubagentStop
        | HookEventKey::ConfigChange
        | HookEventKey::PreCompact => ensure_block_only(
            Harness::Claude,
            event,
            envelope.decision.as_deref(),
            envelope.reason.as_deref(),
        )?,
        HookEventKey::PreToolUse | HookEventKey::PermissionRequest => {
            reject_top_level_decision(Harness::Claude, event, envelope)?
        }
        _ => {
            if envelope.decision.is_some() || envelope.reason.is_some() {
                return Err(invalid(
                    Harness::Claude,
                    event,
                    "top-level decision/reason is not supported for this event",
                ));
            }
        }
    }

    if matches!(event, HookEventKey::PreToolUse)
        && let Some(hso) = &envelope.hook_specific_output
    {
        let Some(decision) = hso.get("permissionDecision").and_then(|v| v.as_str()) else {
            return Err(invalid(
                Harness::Claude,
                event,
                "PreToolUse hookSpecificOutput must include permissionDecision",
            ));
        };

        if !matches!(decision, "allow" | "deny" | "ask" | "defer") {
            return Err(invalid(
                Harness::Claude,
                event,
                "permissionDecision must be one of: allow, deny, ask, defer",
            ));
        }
    }

    if matches!(event, HookEventKey::PermissionRequest)
        && let Some(hso) = &envelope.hook_specific_output
    {
        let Some(decision) = hso.get("decision").and_then(|v| v.as_object()) else {
            return Err(invalid(
                Harness::Claude,
                event,
                "PermissionRequest hookSpecificOutput must include a decision object",
            ));
        };

        let Some(behavior) = decision.get("behavior").and_then(|v| v.as_str()) else {
            return Err(invalid(
                Harness::Claude,
                event,
                "PermissionRequest decision.behavior is required",
            ));
        };

        match behavior {
            "allow" => {
                if decision.get("message").is_some() || decision.get("interrupt").is_some() {
                    return Err(invalid(
                        Harness::Claude,
                        event,
                        "PermissionRequest allow cannot include message or interrupt",
                    ));
                }
            }
            "deny" => {
                if decision.get("updatedInput").is_some()
                    || decision.get("updatedPermissions").is_some()
                {
                    return Err(invalid(
                        Harness::Claude,
                        event,
                        "PermissionRequest deny cannot include updatedInput or updatedPermissions",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    Harness::Claude,
                    event,
                    "PermissionRequest decision.behavior must be allow or deny",
                ));
            }
        }
    }

    if matches!(event, HookEventKey::WorktreeCreate)
        && let Some(path) = envelope
            .hook_specific_output
            .as_ref()
            .and_then(|hso| hso.get("worktreePath"))
            .and_then(|v| v.as_str())
        && !is_absolute(path)
    {
        return Err(invalid(
            Harness::Claude,
            event,
            "worktreePath must be an absolute path",
        ));
    }

    if let Some(paths) = envelope
        .hook_specific_output
        .as_ref()
        .and_then(|hso| hso.get("watchPaths"))
    {
        if !matches!(event, HookEventKey::CwdChanged | HookEventKey::FileChanged) {
            return Err(invalid(
                Harness::Claude,
                event,
                "watchPaths is only supported for CwdChanged and FileChanged",
            ));
        }

        let Some(paths) = paths.as_array() else {
            return Err(invalid(
                Harness::Claude,
                event,
                "watchPaths must be an array",
            ));
        };

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

    if matches!(event, HookEventKey::Elicitation | HookEventKey::ElicitationResult)
        && let Some(hso) = &envelope.hook_specific_output
    {
        let Some(action) = hso.get("action").and_then(|v| v.as_str()) else {
            return Err(invalid(
                Harness::Claude,
                event,
                "elicitation outputs must include action",
            ));
        };

        if !matches!(action, "accept" | "decline" | "cancel") {
            return Err(invalid(
                Harness::Claude,
                event,
                "elicitation action must be accept, decline, or cancel",
            ));
        }

        if let Some(content) = hso.get("content")
            && !content.is_object()
        {
            return Err(invalid(
                Harness::Claude,
                event,
                "elicitation content must be an object",
            ));
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
pub fn validate_gemini(
    event: &HookEventKey,
    envelope: &GeminiEnvelope,
) -> Result<(), HookkitError> {
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

    if matches!(event, HookEventKey::PreToolUse)
        && let Some(tool_input) = envelope
            .hook_specific_output
            .as_ref()
            .and_then(|hso| hso.get("tool_input"))
        && !tool_input.is_object()
    {
        return Err(invalid(
            Harness::Gemini,
            event,
            "hookSpecificOutput.tool_input must be a JSON object",
        ));
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
        assert!(validate_claude(&HookEventKey::WorktreeCreate, &env).is_err());
    }

    #[test]
    fn claude_watch_paths_require_absolute_entries() {
        let env = ClaudeEnvelope::watch_paths(vec!["/tmp/ok".to_string(), "relative".to_string()]);
        assert!(validate_claude(&HookEventKey::FileChanged, &env).is_err());
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
