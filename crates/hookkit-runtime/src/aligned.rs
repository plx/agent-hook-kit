use hookkit_common::{PostToolUseInput, PostToolUseOutput};
use hookkit_core::{EventSpec, HarnessId, HookkitError, ProcessEmission, RawInvocation};

/// Execute aligned PostToolUse logic for an explicitly selected harness.
pub fn execute_post_tool_use<F>(
    harness: HarnessId,
    bytes: impl Into<Vec<u8>>,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(PostToolUseInput, &RawInvocation) -> hookkit_core::Result<PostToolUseOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let input = parse_selected(&harness, &invocation)?;
    let expected = input.harness();
    let output = handler(input, &invocation)?;
    if output.harness() != expected {
        return Err(HookkitError::InvalidForHint {
            harness,
            event: hookkit_core::EventId::builtin("PostToolUse"),
            message: "native input and output arms differ".into(),
        });
    }
    emit(output)
}

fn parse_selected(
    harness: &HarnessId,
    invocation: &RawInvocation,
) -> hookkit_core::Result<PostToolUseInput> {
    match harness.as_str() {
        "claude-code" => match hookkit_claude::input::parse(invocation.json())? {
            hookkit_claude::ClaudeHookInput::PostToolUse(input) => {
                Ok(PostToolUseInput::Claude(input))
            }
            _ => invalid(harness.clone(), "expected Claude PostToolUse"),
        },
        "codex" => match hookkit_codex::input::parse(invocation.json())? {
            hookkit_codex::CodexHookInput::PostToolUse(input) => Ok(PostToolUseInput::Codex(input)),
            _ => invalid(harness.clone(), "expected Codex PostToolUse"),
        },
        "gemini-cli" => match hookkit_gemini::input::parse(invocation.json())? {
            hookkit_gemini::GeminiHookInput::AfterTool(input) => {
                Ok(PostToolUseInput::Gemini(input))
            }
            _ => invalid(harness.clone(), "expected Gemini AfterTool"),
        },
        "antigravity" => {
            hookkit_antigravity::PostToolUse::parse(invocation).map(PostToolUseInput::Antigravity)
        }
        _ => Err(HookkitError::NoEventCandidate {
            harness: harness.clone(),
        }),
    }
}

fn invalid<T>(harness: HarnessId, message: &str) -> hookkit_core::Result<T> {
    Err(HookkitError::InvalidForHint {
        harness,
        event: hookkit_core::EventId::builtin("PostToolUse"),
        message: message.into(),
    })
}

fn emit(output: PostToolUseOutput) -> hookkit_core::Result<ProcessEmission> {
    Ok(match output {
        PostToolUseOutput::Claude(output) => match output {
            hookkit_claude::ClaudeHookOutput::Empty => ProcessEmission::success_empty(),
            hookkit_claude::ClaudeHookOutput::Json(value) => ProcessEmission::success_json(&value)?,
            hookkit_claude::ClaudeHookOutput::BlockingError { stderr } => {
                ProcessEmission::protocol_error(stderr, 2)
            }
        },
        PostToolUseOutput::Codex(output) => match output {
            hookkit_codex::CodexHookOutput::Empty => ProcessEmission::success_empty(),
            hookkit_codex::CodexHookOutput::Json(value) => ProcessEmission::success_json(&value)?,
            hookkit_codex::CodexHookOutput::BlockingDeny { stderr } => {
                ProcessEmission::protocol_error(stderr, 2)
            }
        },
        PostToolUseOutput::Gemini(output) => match output {
            hookkit_gemini::GeminiHookOutput::Empty => ProcessEmission::success_empty(),
            hookkit_gemini::GeminiHookOutput::Json(value) => ProcessEmission::success_json(&value)?,
            hookkit_gemini::GeminiHookOutput::BlockingError { stderr } => {
                ProcessEmission::protocol_error(stderr, 2)
            }
        },
        PostToolUseOutput::Antigravity(output) => hookkit_antigravity::PostToolUse::emit(output)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatched_arms_fail_before_emission() {
        let bytes = br#"{"session_id":"s","cwd":"/repo","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{},"tool_response":{}}"#.to_vec();
        let result = execute_post_tool_use(HarnessId::CLAUDE_CODE, bytes, |_, _| {
            Ok(PostToolUseOutput::Codex(
                hookkit_codex::CodexHookOutput::Empty,
            ))
        });
        assert!(matches!(result, Err(HookkitError::InvalidForHint { .. })));
    }

    #[test]
    fn antigravity_clean_path_needs_no_fabricated_tool_data() {
        let bytes = br#"{"conversationId":"c","workspacePaths":["/repo","/lib"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","stepIdx":2}"#.to_vec();
        let emission = execute_post_tool_use(HarnessId::ANTIGRAVITY, bytes, |input, _| {
            assert_eq!(input.workspace_roots().unwrap().len(), 2);
            Ok(PostToolUseOutput::Antigravity(Default::default()))
        })
        .unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }
}
