use crate::{EventId, NativeContext};
use std::collections::BTreeMap;
use std::fmt;

/// A deterministic, UTF-8 view of only the environment variables declared by
/// a hook environment specification.
///
/// Values are deliberately omitted from `Debug`: plugin options and other
/// inherited hook state can contain secrets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EnvironmentVariables(BTreeMap<String, String>);

impl EnvironmentVariables {
    /// Creates an empty variable set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Collects name/value pairs, replacing earlier duplicate names.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self(
            pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        )
    }

    /// Inserts a variable and returns its previous value, if any.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) -> Option<String> {
        self.0.insert(name.into(), value.into())
    }

    /// Returns a variable's value.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Reports whether the set contains `name`.
    pub fn contains_key(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Iterates through names and values in lexicographic name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Iterates through variable names in lexicographic order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// Reports whether no variables were captured.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of captured variables.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for EnvironmentVariables {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentVariables")
            .field("names", &self.0.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl<K, V> FromIterator<(K, V)> for EnvironmentVariables
where
    K: Into<String>,
    V: Into<String>,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self::from_pairs(iter)
    }
}

impl From<BTreeMap<String, String>> for EnvironmentVariables {
    fn from(values: BTreeMap<String, String>) -> Self {
        Self(values)
    }
}

impl From<EnvironmentVariables> for BTreeMap<String, String> {
    fn from(values: EnvironmentVariables) -> Self {
        values.0
    }
}

/// Harness-native environment contract for command-hook processes.
///
/// Implementations parse a deterministic map so tests never need to mutate
/// the process environment. The runtime uses `VARIABLE_NAMES` and
/// `VARIABLE_PREFIXES` to capture only declared variables before calling the
/// same parser.
pub trait CommandEnvironmentSpec: Sized {
    /// Exact environment variable names that may carry native hook state.
    const VARIABLE_NAMES: &'static [&'static str];

    /// Dynamic native prefixes, such as Claude plugin option variables.
    ///
    /// Prefixes match whatever the parent process happened to export, so the
    /// runtime skips a prefix-matched variable whose value is not UTF-8
    /// instead of failing the invocation.
    const VARIABLE_PREFIXES: &'static [&'static str] = &[];

    /// Names from [`Self::VARIABLE_NAMES`] that unrelated software may also
    /// export, such as generic `PLUGIN_ROOT`-style names.
    ///
    /// The runtime skips one of these whose value is not UTF-8, as it does
    /// for prefix-matched variables, so stray inherited state cannot disable
    /// every hook. Any other declared name with a non-UTF-8 value is an
    /// error. The default is empty.
    const LENIENT_VARIABLE_NAMES: &'static [&'static str] = &[];

    /// Parses a captured variable set for `event`.
    ///
    /// The map contains only names selected by [`Self::VARIABLE_NAMES`] and
    /// [`Self::VARIABLE_PREFIXES`]. Implementations should reject malformed
    /// values but may treat declared, absent variables as optional.
    fn from_variables(event: &EventId, variables: &EnvironmentVariables) -> crate::Result<Self>;

    /// Cross-check redundant environment and native-input values.
    fn validate_context(&self, _event: &EventId, _context: &NativeContext) -> crate::Result<()> {
        Ok(())
    }
}

/// Environment contract for test or application events that declare no
/// harness-provided process state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoCommandEnvironment;

impl CommandEnvironmentSpec for NoCommandEnvironment {
    const VARIABLE_NAMES: &'static [&'static str] = &[];

    fn from_variables(_event: &EventId, _variables: &EnvironmentVariables) -> crate::Result<Self> {
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_construction_and_lookup_are_deterministic() {
        let variables = EnvironmentVariables::from_pairs([("B", "2"), ("A", "1")]);
        assert_eq!(variables.get("A"), Some("1"));
        assert_eq!(variables.names().collect::<Vec<_>>(), ["A", "B"]);
    }

    #[test]
    fn debug_redacts_values() {
        let variables = EnvironmentVariables::from_pairs([("TOKEN", "super-secret")]);
        let rendered = format!("{variables:?}");
        assert!(rendered.contains("TOKEN"));
        assert!(!rendered.contains("super-secret"));
    }
}
