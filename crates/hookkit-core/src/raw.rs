use serde::{Deserialize, Serialize};

/// Wrapper preserving the original JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPayload(pub serde_json::Value);

impl RawPayload {
    /// Access the inner JSON value.
    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }

    /// Consume the wrapper, returning the inner JSON value.
    pub fn into_value(self) -> serde_json::Value {
        self.0
    }
}

impl From<serde_json::Value> for RawPayload {
    fn from(v: serde_json::Value) -> Self {
        RawPayload(v)
    }
}
