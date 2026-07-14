//! Gemini CLI command-hook environment contract.
//!
//! Gemini exports the same five values for every current command-hook event.
//! Values are preserved exactly because a command hook's configured `env`
//! overrides the built-ins after Gemini constructs them.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError, Utf8PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiCommandEnvironment {
    pub project_dir: Utf8PathBuf,
    pub plans_dir: Utf8PathBuf,
    pub cwd: Utf8PathBuf,
    pub session_id: String,
    pub claude_project_dir: Utf8PathBuf,
}

impl GeminiCommandEnvironment {
    pub fn from_map(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        <Self as CommandEnvironmentSpec>::from_variables(event, variables)
    }
}

impl CommandEnvironmentSpec for GeminiCommandEnvironment {
    const VARIABLE_NAMES: &'static [&'static str] = &[
        "GEMINI_PROJECT_DIR",
        "GEMINI_PLANS_DIR",
        "GEMINI_CWD",
        "GEMINI_SESSION_ID",
        "CLAUDE_PROJECT_DIR",
    ];

    fn from_variables(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        if event.harness() != &HarnessId::GEMINI_CLI {
            return Err(HookkitError::EventHarnessMismatch {
                harness: HarnessId::GEMINI_CLI,
                event: event.clone(),
            });
        }

        let project_dir = required(event, variables, "GEMINI_PROJECT_DIR")?;
        let plans_dir = required(event, variables, "GEMINI_PLANS_DIR")?;
        let cwd = required(event, variables, "GEMINI_CWD")?;
        let session_id = required(event, variables, "GEMINI_SESSION_ID")?;
        let claude_project_dir = required(event, variables, "CLAUDE_PROJECT_DIR")?;
        Ok(Self {
            project_dir: project_dir.into(),
            plans_dir: plans_dir.into(),
            cwd: cwd.into(),
            session_id: session_id.into(),
            claude_project_dir: claude_project_dir.into(),
        })
    }
}

fn required<'a>(
    event: &EventId,
    variables: &'a EnvironmentVariables,
    name: &'static str,
) -> hookkit_core::Result<&'a str> {
    variables
        .get(name)
        .ok_or_else(|| invalid(event, format!("missing required {name}")))
}

fn invalid(event: &EventId, message: impl Into<String>) -> HookkitError {
    HookkitError::InvalidHookEnvironment {
        event: event.clone(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &[&str] = &[
        "SessionStart",
        "BeforeAgent",
        "BeforeModel",
        "BeforeToolSelection",
        "BeforeTool",
        "AfterTool",
        "AfterModel",
        "AfterAgent",
        "Notification",
        "PreCompress",
        "SessionEnd",
    ];

    fn event(name: &'static str) -> EventId {
        EventId::builtin(HarnessId::GEMINI_CLI, name)
    }

    fn variables() -> EnvironmentVariables {
        EnvironmentVariables::from_pairs([
            ("GEMINI_PROJECT_DIR", "/repo"),
            ("GEMINI_PLANS_DIR", "/repo/.gemini/plans"),
            ("GEMINI_CWD", "/repo"),
            ("GEMINI_SESSION_ID", "session-1"),
            ("CLAUDE_PROJECT_DIR", "/repo"),
        ])
    }

    #[test]
    fn every_documented_event_uses_the_common_environment() {
        for name in EVENTS {
            let environment =
                GeminiCommandEnvironment::from_map(&event(name), &variables()).unwrap();
            assert_eq!(environment.session_id, "session-1");
            assert_eq!(environment.plans_dir, "/repo/.gemini/plans");
        }
    }

    #[test]
    fn keys_are_required_but_configured_overrides_are_preserved() {
        let mut missing = variables();
        let mut raw: std::collections::BTreeMap<String, String> = missing.clone().into();
        raw.remove("GEMINI_SESSION_ID");
        missing = raw.into();
        assert!(matches!(
            GeminiCommandEnvironment::from_map(&event("BeforeTool"), &missing),
            Err(HookkitError::InvalidHookEnvironment { .. })
        ));

        let mut overridden = variables();
        overridden.insert("CLAUDE_PROJECT_DIR", "/other");
        overridden.insert("GEMINI_SESSION_ID", "");
        let environment =
            GeminiCommandEnvironment::from_map(&event("AfterTool"), &overridden).unwrap();
        assert_eq!(environment.claude_project_dir, "/other");
        assert!(environment.session_id.is_empty());
    }
}
