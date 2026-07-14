//! Antigravity command-hook environment contract.
//!
//! The current official hook reference defines all native invocation state in
//! the JSON payload and does not define any Antigravity-provided environment
//! variables. Keeping an explicit zero-field type prevents inherited ambient
//! variables from being mistaken for protocol state.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AntigravityCommandEnvironment;

impl AntigravityCommandEnvironment {
    pub fn from_map(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        <Self as CommandEnvironmentSpec>::from_variables(event, variables)
    }
}

impl CommandEnvironmentSpec for AntigravityCommandEnvironment {
    const VARIABLE_NAMES: &'static [&'static str] = &[];

    fn from_variables(
        event: &EventId,
        _variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        if event.harness() != &HarnessId::ANTIGRAVITY {
            return Err(HookkitError::EventHarnessMismatch {
                harness: HarnessId::ANTIGRAVITY,
                event: event.clone(),
            });
        }
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::HarnessId;

    #[test]
    fn every_documented_event_has_explicitly_empty_native_environment() {
        for name in [
            "PreToolUse",
            "PostToolUse",
            "PreInvocation",
            "PostInvocation",
            "Stop",
        ] {
            let event = EventId::builtin(HarnessId::ANTIGRAVITY, name);
            let variables = EnvironmentVariables::from_pairs([("PATH", "/bin")]);
            assert_eq!(
                AntigravityCommandEnvironment::from_map(&event, &variables).unwrap(),
                AntigravityCommandEnvironment
            );
        }
    }
}
