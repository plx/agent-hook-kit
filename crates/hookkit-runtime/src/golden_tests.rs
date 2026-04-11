//! Golden output tests — verify serialized outputs match stable snapshots.

#[cfg(test)]
mod tests {
    use hookkit_claude::output::OutputEnvelope as ClaudeEnvelope;
    use hookkit_codex::output::OutputEnvelope as CodexEnvelope;
    use hookkit_gemini::output::OutputEnvelope as GeminiEnvelope;

    fn load_golden(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/fixtures/golden/{name}",
            env!("CARGO_MANIFEST_DIR").replace("/crates/hookkit-runtime", "")
        );
        let data = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read golden fixture {path}: {e}"));
        serde_json::from_str(&data).expect("golden fixture is not valid JSON")
    }

    // --- Claude golden tests ---

    #[test]
    fn golden_claude_pre_tool_deny() {
        let out = ClaudeEnvelope::pre_tool_deny("destructive command blocked");
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("claude_pre_tool_deny.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn golden_claude_post_tool_context() {
        let out = ClaudeEnvelope::with_context("formatted 3 files with rustfmt");
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("claude_post_tool_context.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn golden_claude_stop_continue() {
        let out = ClaudeEnvelope::stop_continue("tests still failing, keep going");
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("claude_stop_continue.json");
        assert_eq!(actual, expected);
    }

    // --- Codex golden tests ---

    #[test]
    fn golden_codex_pre_tool_deny() {
        let out = CodexEnvelope::deny("force push to main not allowed");
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("codex_pre_tool_deny.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn golden_codex_stop_continue() {
        let out = CodexEnvelope::stop_continue("task incomplete");
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("codex_stop_continue.json");
        assert_eq!(actual, expected);
    }

    // --- Gemini golden tests ---

    #[test]
    fn golden_gemini_before_tool_rewrite() {
        let out =
            GeminiEnvelope::rewrite_tool_input(serde_json::json!({"command": ["echo", "safe"]}));
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("gemini_before_tool_rewrite.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn golden_gemini_after_tool_replace() {
        let out = GeminiEnvelope::replace_tool_result(serde_json::json!({
            "stdout": "replaced output",
            "exitCode": 0
        }));
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("gemini_after_tool_replace.json");
        assert_eq!(actual, expected);
    }

    #[test]
    fn golden_gemini_after_agent_retry() {
        let out = GeminiEnvelope::retry("output did not match expectations");
        let actual = serde_json::to_value(&out).unwrap();
        let expected = load_golden("gemini_after_agent_retry.json");
        assert_eq!(actual, expected);
    }
}
