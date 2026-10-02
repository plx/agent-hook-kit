/// Lossless invocation retaining both exact bytes and parsed JSON.
#[derive(Debug, Clone)]
pub struct RawInvocation {
    bytes: Vec<u8>,
    json: serde_json::Value,
    source: Option<String>,
}

impl RawInvocation {
    /// Deepest nesting of JSON arrays and objects that [`Self::parse`]
    /// accepts. serde_json's default recursion limit of 128 rejects the
    /// 128th nested level.
    pub const MAX_NESTING_DEPTH: usize = 127;

    /// Parses JSON while retaining the exact input bytes.
    ///
    /// The input may be any JSON value; native event parsers impose their own
    /// object-shape requirements later. Leading and trailing JSON whitespace
    /// is accepted, but a byte-order mark, trailing data, or empty input is
    /// rejected.
    ///
    /// JavaScript harnesses can emit `\uXXXX` escapes for unpaired UTF-16
    /// surrogates, for example when a long string is truncated between the two
    /// halves of an emoji. Strict JSON parsers reject those escapes, so when
    /// the first parse fails, each unpaired surrogate escape is replaced with
    /// `\uFFFD` (U+FFFD REPLACEMENT CHARACTER, the same six bytes long) and
    /// the input is parsed once more. [`Self::json`] then contains U+FFFD in
    /// place of each unpaired surrogate, while [`Self::bytes`] still returns
    /// the input exactly as received.
    ///
    /// Nesting is limited to [`Self::MAX_NESTING_DEPTH`] arrays and objects
    /// (serde_json's recursion limit), which keeps a hostile payload from
    /// overflowing the stack. Claude Code and Codex impose no such limit, and
    /// an MCP tool's `tool_input` is arbitrary model-generated JSON, so a
    /// deeper payload is valid for the harness but is rejected here with
    /// [`crate::HookkitError::InvalidJson`] (`recursion limit exceeded`). The
    /// hook then never reaches its handler and is reported under the stdin
    /// runner's failure policy: by default the pending action proceeds, and a
    /// fail-closed guard denies it.
    pub fn parse(bytes: impl Into<Vec<u8>>) -> crate::Result<Self> {
        let bytes = bytes.into();
        let json = match serde_json::from_slice(&bytes) {
            Ok(json) => json,
            Err(error) => match replace_unpaired_surrogate_escapes(&bytes) {
                Some(repaired) => serde_json::from_slice(&repaired)?,
                None => return Err(error.into()),
            },
        };
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

const REPLACEMENT_ESCAPE: &[u8; 6] = br"\uFFFD";

/// Returns a copy of `bytes` in which every `\uXXXX` escape that encodes an
/// unpaired UTF-16 surrogate inside a JSON string is replaced by `\uFFFD`, or
/// `None` when there is no such escape.
///
/// The scan tracks JSON string and escape state, so an escaped backslash
/// followed by `u` (the literal text `\ud83d`) is left alone. Replacements
/// have the same length as the escape they replace, so byte offsets in any
/// later parse error still point into the original input.
fn replace_unpaired_surrogate_escapes(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut repaired: Option<Vec<u8>> = None;
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !in_string {
            in_string = byte == b'"';
            index += 1;
            continue;
        }
        match byte {
            b'"' => {
                in_string = false;
                index += 1;
            }
            b'\\' => {
                let Some(unit) = unicode_escape(bytes, index) else {
                    // A two-byte escape such as `\\` or `\"`, or a malformed
                    // escape that the parser will report on its own.
                    index += 2;
                    continue;
                };
                let high = (0xD800..=0xDBFF).contains(&unit);
                let low = (0xDC00..=0xDFFF).contains(&unit);
                if high && unicode_escape(bytes, index + 6).is_some_and(is_low_surrogate) {
                    index += 12;
                    continue;
                }
                if high || low {
                    repaired.get_or_insert_with(|| bytes.to_vec())[index..index + 6]
                        .copy_from_slice(REPLACEMENT_ESCAPE);
                }
                index += 6;
            }
            _ => index += 1,
        }
    }
    repaired
}

/// Decodes the `\uXXXX` escape starting at `index`, if there is one.
fn unicode_escape(bytes: &[u8], index: usize) -> Option<u16> {
    let escape = bytes.get(index..index + 6)?;
    if escape[0] != b'\\' || escape[1] != b'u' {
        return None;
    }
    let digits = std::str::from_utf8(&escape[2..]).ok()?;
    if !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(digits, 16).ok()
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> crate::Result<RawInvocation> {
        RawInvocation::parse(text.as_bytes().to_vec())
    }

    #[test]
    fn unpaired_high_surrogate_is_replaced_and_bytes_are_kept() {
        let text = r#"{"error":"ab\ud83d... [3 characters truncated] ..."}"#;
        let raw = parse(text).unwrap();
        assert_eq!(
            raw.json()["error"],
            "ab\u{FFFD}... [3 characters truncated] ..."
        );
        assert_eq!(raw.bytes(), text.as_bytes());
    }

    #[test]
    fn unpaired_low_surrogate_is_replaced() {
        let raw = parse(r#"{"e":"\ude00tail"}"#).unwrap();
        assert_eq!(raw.json()["e"], "\u{FFFD}tail");
    }

    #[test]
    fn reversed_pair_replaces_both_halves() {
        let raw = parse(r#"{"e":"\ude00\ud83d"}"#).unwrap();
        assert_eq!(raw.json()["e"], "\u{FFFD}\u{FFFD}");
    }

    #[test]
    fn high_surrogate_followed_by_a_non_surrogate_escape_is_replaced() {
        let raw = parse(r#"{"e":"\ud83d\n\u0041"}"#).unwrap();
        assert_eq!(raw.json()["e"], "\u{FFFD}\nA");
    }

    #[test]
    fn paired_surrogates_decode_normally() {
        let raw = parse(r#"{"e":"\ud83d\ude00"}"#).unwrap();
        assert_eq!(raw.json()["e"], "\u{1F600}");
    }

    #[test]
    fn escaped_backslash_before_u_is_literal_text() {
        let raw = parse(r#"{"e":"\\ud83d","f":"\ud83d"}"#).unwrap();
        assert_eq!(raw.json()["e"], r"\ud83d");
        assert_eq!(raw.json()["f"], "\u{FFFD}");
    }

    #[test]
    fn surrogate_escapes_in_object_keys_are_repaired_too() {
        let raw = parse(r#"{"k\ud800":1}"#).unwrap();
        assert_eq!(raw.json()["k\u{FFFD}"], 1);
    }

    #[test]
    fn other_errors_are_still_reported_after_repair() {
        let error = parse(r#"{"e":"\ud83d""#).unwrap_err();
        assert!(matches!(error, crate::HookkitError::InvalidJson(_)));
    }

    #[test]
    fn edge_inputs_are_pinned() {
        assert!(parse("").is_err(), "empty input is not JSON");
        assert!(
            RawInvocation::parse(b"\xEF\xBB\xBF{}".to_vec()).is_err(),
            "a UTF-8 byte-order mark is rejected"
        );
        assert!(parse("{}{}").is_err(), "trailing data is rejected");
        assert!(parse(" {}\n").is_ok(), "surrounding whitespace is accepted");
        assert_eq!(parse("null").unwrap().json(), &serde_json::Value::Null);
    }

    /// A Codex-shaped `PreToolUse` payload whose MCP `tool_input` nests
    /// `depth` arrays inside its outer object.
    fn nested_tool_input(depth: usize) -> String {
        format!(
            r#"{{"hook_event_name":"PreToolUse","tool_name":"mcp__x__y","tool_input":{{"a":{}1{}}}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        )
    }

    #[test]
    fn nesting_is_limited_to_serde_jsons_recursion_limit() {
        // The payload object and `tool_input` are two levels themselves.
        let deepest = RawInvocation::MAX_NESTING_DEPTH - 2;
        let raw = parse(&nested_tool_input(deepest)).unwrap();
        assert_eq!(raw.json()["tool_name"], "mcp__x__y");

        let error = parse(&nested_tool_input(deepest + 1)).unwrap_err();
        assert!(
            matches!(&error, crate::HookkitError::InvalidJson(source)
                if source.to_string().starts_with("recursion limit exceeded")),
            "{error}"
        );
        // Deeper payloads fail the same way instead of overflowing the stack.
        assert!(parse(&nested_tool_input(100_000)).is_err());
    }

    #[test]
    fn valid_input_is_never_rewritten() {
        assert!(replace_unpaired_surrogate_escapes(br#"{"a":"\u0041\ud83d\ude00\\u"}"#).is_none());
    }
}
