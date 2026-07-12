//! Contract-first event specifications for the selected catalog snapshot.

use hookkit_core::{EventCategory, EventId, EventSpec, HarnessId, ProcessEmission, RawInvocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_ID: &str = "docs-2026-07-12-r1";
pub static EVENTS: &[hookkit_core::NativeEventDescriptor] = &[
    hookkit_core::NativeEventDescriptor {
        contract_id: "claude-code/docs-2026-07-12-r1/SessionStart",
        harness: "claude-code",
        event: "SessionStart",
        native_input: true,
        native_output: true,
        bindings: &[hookkit_core::HandlerKind::Command],
        conformance_cases: &["command-structured", "command-text"],
    },
    hookkit_core::NativeEventDescriptor {
        contract_id: "claude-code/docs-2026-07-12-r1/WorktreeCreate",
        harness: "claude-code",
        event: "WorktreeCreate",
        native_input: true,
        native_output: true,
        bindings: &[hookkit_core::HandlerKind::Command],
        conformance_cases: &["command-created", "command-failed"],
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub source: SessionSource,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionSource {
    Startup,
    Resume,
    Clear,
    Compact,
}

#[derive(Debug, Clone)]
pub enum SessionStartOutput {
    Structured(StructuredSessionStartOutput),
    Text(String),
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredSessionStartOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<SessionStartSpecific>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionStartSpecific {
    hook_event_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reload_skills: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_title: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    watch_paths: Vec<hookkit_core::Utf8PathBuf>,
}

impl SessionStartOutput {
    pub fn no_op() -> Self {
        Self::Structured(StructuredSessionStartOutput::default())
    }

    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredSessionStartOutput {
            hook_specific_output: Some(SessionStartSpecific {
                hook_event_name: "SessionStart",
                additional_context: Some(context.into()),
                reload_skills: None,
                session_title: None,
                watch_paths: Vec::new(),
            }),
            system_message: None,
        })
    }

    pub fn text_context(context: impl Into<String>) -> Self {
        Self::Text(context.into())
    }

    pub fn structured(
        context: Option<String>,
        reload_skills: Option<bool>,
        session_title: Option<String>,
        watch_paths: Vec<hookkit_core::Utf8PathBuf>,
    ) -> Self {
        Self::Structured(StructuredSessionStartOutput {
            hook_specific_output: Some(SessionStartSpecific {
                hook_event_name: "SessionStart",
                additional_context: context,
                reload_skills,
                session_title,
                watch_paths,
            }),
            system_message: None,
        })
    }
}

pub enum SessionStart {}

impl EventSpec for SessionStart {
    type Input = SessionStartInput;
    type CommandOutput = SessionStartOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const EVENT: EventId = EventId::builtin("SessionStart");
    const CATEGORY: EventCategory = EventCategory::Session;
    const CONTRACT_ID: &'static str = "claude-code/docs-2026-07-12-r1/SessionStart";

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "SessionStart")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            SessionStartOutput::Structured(output) => ProcessEmission::success_json(&output),
            SessionStartOutput::Text(context) => Ok(ProcessEmission::success_text(context)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreateInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum WorktreeCreateOutput {
    Created {
        path: hookkit_core::Utf8PathBuf,
        trailing_newline: bool,
    },
    Failed {
        message: String,
        exit_code: u8,
    },
}

impl WorktreeCreateOutput {
    pub fn path(path: hookkit_core::Utf8PathBuf) -> hookkit_core::Result<Self> {
        if !path.is_absolute() {
            return Err(hookkit_core::HookkitError::InvalidOutputCombination {
                harness: hookkit_core::Harness::Claude,
                event: hookkit_core::HookEventKey::WorktreeCreate,
                message: "worktree path must be absolute".into(),
            });
        }
        Ok(Self::Created {
            path,
            trailing_newline: false,
        })
    }

    pub fn path_with_newline(path: hookkit_core::Utf8PathBuf) -> hookkit_core::Result<Self> {
        let output = Self::path(path)?;
        Ok(match output {
            Self::Created { path, .. } => Self::Created {
                path,
                trailing_newline: true,
            },
            Self::Failed { .. } => unreachable!(),
        })
    }

    pub fn failed(message: impl Into<String>, exit_code: u8) -> hookkit_core::Result<Self> {
        if exit_code == 0 {
            return Err(hookkit_core::HookkitError::InvalidOutputCombination {
                harness: hookkit_core::Harness::Claude,
                event: hookkit_core::HookEventKey::WorktreeCreate,
                message: "worktree failure exit code must be nonzero".into(),
            });
        }
        Ok(Self::Failed {
            message: message.into(),
            exit_code,
        })
    }
}

pub enum WorktreeCreate {}

impl EventSpec for WorktreeCreate {
    type Input = WorktreeCreateInput;
    type CommandOutput = WorktreeCreateOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const EVENT: EventId = EventId::builtin("WorktreeCreate");
    const CATEGORY: EventCategory = EventCategory::Worktree;
    const CONTRACT_ID: &'static str = "claude-code/docs-2026-07-12-r1/WorktreeCreate";

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "WorktreeCreate")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        Ok(match output {
            WorktreeCreateOutput::Created {
                path,
                trailing_newline,
            } => {
                let mut bytes = path.as_str().as_bytes().to_vec();
                if trailing_newline {
                    bytes.push(b'\n');
                }
                ProcessEmission::success_text(bytes)
            }
            WorktreeCreateOutput::Failed { message, exit_code } => {
                ProcessEmission::protocol_error(message, exit_code)
            }
        })
    }
}

fn require_event(invocation: &RawInvocation, expected: &str) -> hookkit_core::Result<()> {
    let actual = invocation
        .json()
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;
    if actual != expected {
        return Err(hookkit_core::HookkitError::UnknownEvent {
            harness: hookkit_core::Harness::Claude,
            event_name: actual.into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_discriminator_is_fixed_by_output_type() {
        let emission = SessionStart::emit(SessionStartOutput::with_context("ctx")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
    }

    #[test]
    fn session_start_text_is_not_json_or_newline_normalized() {
        let emission = SessionStart::emit(SessionStartOutput::text_context("context")).unwrap();
        assert_eq!(emission.stdout(), b"context");
    }

    #[test]
    fn worktree_command_is_absolute_plain_text_without_newline() {
        let output = WorktreeCreateOutput::path("/tmp/worktree".into()).unwrap();
        let emission = WorktreeCreate::emit(output).unwrap();
        assert_eq!(emission.stdout(), b"/tmp/worktree");
        assert!(!emission.stdout().ends_with(b"\n"));
    }

    #[test]
    fn worktree_failure_is_stderr_only() {
        let output = WorktreeCreateOutput::failed("cannot create", 7).unwrap();
        let emission = WorktreeCreate::emit(output).unwrap();
        assert!(emission.stdout().is_empty());
        assert_eq!(emission.stderr(), b"cannot create");
        assert_eq!(emission.exit_code(), 7);
    }
}
