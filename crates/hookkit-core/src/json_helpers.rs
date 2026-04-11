//! JSON utility helpers for hookkit.

/// Extract a string field from a JSON value.
pub fn get_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(|v| v.as_str())
}

/// Extract a string field, returning an error if missing.
pub fn require_str<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a str, crate::HookkitError> {
    get_str(value, key).ok_or(crate::HookkitError::MissingHookEventName)
}

/// Check if a JSON value is an empty object `{}`.
pub fn is_empty_object(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|m| m.is_empty())
}
