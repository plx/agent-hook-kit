use crate::{EventId, HarnessId, RawInvocation};

/// Cross-harness lifecycle category. This is never an exact protocol identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    Session,
    Prompt,
    Tool,
    Model,
    Agent,
    Context,
    Worktree,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerKind {
    Command,
    Http,
    McpTool,
    Prompt,
    Agent,
}

/// Machine-enumerable implementation declaration exposed by each native crate.
#[derive(Debug, Clone, Copy)]
pub struct NativeEventDescriptor {
    pub contract_id: &'static str,
    pub harness: &'static str,
    pub event: &'static str,
    pub native_input: bool,
    pub native_output: bool,
    pub bindings: &'static [HandlerKind],
    pub conformance_cases: &'static [&'static str],
}

/// Fully materialized process result. Bytes are retained exactly: empty stdout
/// is distinct from JSON null and text is not normalized with a newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEmission {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: u8,
}

impl ProcessEmission {
    pub fn new(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: u8) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    pub fn success_empty() -> Self {
        Self::new(Vec::new(), Vec::new(), 0)
    }

    pub fn success_json<T: serde::Serialize>(value: &T) -> crate::Result<Self> {
        Ok(Self::new(serde_json::to_vec(value)?, Vec::new(), 0))
    }

    pub fn success_text(value: impl Into<Vec<u8>>) -> Self {
        Self::new(value.into(), Vec::new(), 0)
    }

    pub fn protocol_error(message: impl Into<Vec<u8>>, exit_code: u8) -> Self {
        Self::new(Vec::new(), message.into(), exit_code)
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

/// Exact event contract. The associated output prevents one event's response
/// from being returned by another event's typed runner.
pub trait EventSpec {
    type Input;
    type CommandOutput;

    const HARNESS: HarnessId;
    const EVENT: EventId;
    const CATEGORY: EventCategory;
    const CONTRACT_ID: &'static str;

    fn parse(invocation: &RawInvocation) -> crate::Result<Self::Input>;
    fn emit(output: Self::CommandOutput) -> crate::Result<ProcessEmission>;
}
