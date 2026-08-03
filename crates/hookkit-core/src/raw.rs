/// Lossless invocation retaining both exact bytes and parsed JSON.
#[derive(Debug, Clone)]
pub struct RawInvocation {
    bytes: Vec<u8>,
    json: serde_json::Value,
    source: Option<String>,
}

impl RawInvocation {
    /// Parses JSON while retaining the exact input bytes.
    ///
    /// The input may be any JSON value; native event parsers impose their own
    /// object-shape requirements later.
    pub fn parse(bytes: impl Into<Vec<u8>>) -> crate::Result<Self> {
        let bytes = bytes.into();
        let json = serde_json::from_slice(&bytes)?;
        Ok(Self {
            bytes,
            json,
            source: None,
        })
    }

    /// Attaches a human-readable input source such as a file or stream name.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Returns the exact bytes passed to [`Self::parse`].
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the parsed JSON value.
    pub fn json(&self) -> &serde_json::Value {
        &self.json
    }

    /// Returns the optional human-readable source label.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}
