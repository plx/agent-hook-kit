use serde::{Deserialize, Serialize};

/// Lossless invocation retaining both exact bytes and parsed JSON.
#[derive(Debug, Clone)]
pub struct RawInvocation {
    bytes: Vec<u8>,
    json: serde_json::Value,
    source: Option<String>,
}

impl RawInvocation {
    pub fn parse(bytes: impl Into<Vec<u8>>) -> crate::Result<Self> {
        let bytes = bytes.into();
        let json = serde_json::from_slice(&bytes)?;
        Ok(Self {
            bytes,
            json,
            source: None,
        })
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn json(&self) -> &serde_json::Value {
        &self.json
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

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
