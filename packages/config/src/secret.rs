//! Secret references, resolution, and the redacted view.
//!
//! A configuration file never contains a secret value — only a [`SecretRef`]. Values
//! are resolved at startup from the environment or a secret manager and wrapped in
//! [`secrecy::SecretString`], which zeroises on drop and refuses to print.

use std::collections::HashMap;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// A reference to a secret held elsewhere, e.g. `lynx/production/postgres/app`.
///
/// The reference itself is not sensitive and is safe to log and to show in an admin UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// The secret's path in the store.
    pub secret_ref: String,
}

impl SecretRef {
    /// Create a reference.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            secret_ref: path.into(),
        }
    }

    /// The store path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.secret_ref
    }
}

/// An error resolving a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// No value was found for the reference.
    #[error("secret not found: {0}")]
    NotFound(String),
}

/// A source of secret values, keyed by reference path.
pub trait SecretSource {
    /// Look up a secret by path. Returns `None` when absent.
    fn get(&self, path: &str) -> Option<SecretString>;
}

impl SecretRef {
    /// Resolve this reference against a source.
    ///
    /// # Errors
    /// Returns [`SecretError::NotFound`] when the source has no value for the path.
    pub fn resolve(&self, source: &dyn SecretSource) -> Result<SecretString, SecretError> {
        source
            .get(self.path())
            .ok_or_else(|| SecretError::NotFound(self.path().to_string()))
    }
}

/// An in-memory [`SecretSource`], for tests and for the environment loader's output.
#[derive(Debug, Default)]
pub struct MemorySecretSource {
    values: HashMap<String, SecretString>,
}

impl MemorySecretSource {
    /// An empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a value.
    pub fn insert(&mut self, path: impl Into<String>, value: impl Into<String>) {
        self.values
            .insert(path.into(), SecretString::from(value.into()));
    }
}

impl SecretSource for MemorySecretSource {
    fn get(&self, path: &str) -> Option<SecretString> {
        self.values.get(path).cloned()
    }
}

/// A view of a value that always renders as `[redacted]`.
///
/// Wrap anything secret before it reaches a logger or an admin response.
pub struct Redacted<'a, T>(pub &'a T);

impl<T> std::fmt::Debug for Redacted<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

impl<T> std::fmt::Display for Redacted<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Render a secret as its redaction marker, without exposing it.
#[must_use]
pub fn redacted_string(_secret: &SecretString) -> String {
    "[redacted]".to_string()
}

/// Expose a resolved secret for the one place that legitimately needs it (a connection
/// string passed to a driver). Kept as a named function so its call sites are greppable.
#[must_use]
pub fn expose(secret: &SecretString) -> &str {
    secret.expose_secret()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> MemorySecretSource {
        let mut s = MemorySecretSource::new();
        s.insert("db", "postgres://user:pw@host/db");
        s
    }

    #[test]
    fn resolves_present_secret() {
        let r = SecretRef::new("db");
        let value = r.resolve(&source()).unwrap();
        assert_eq!(expose(&value), "postgres://user:pw@host/db");
    }

    #[test]
    fn missing_secret_is_an_error() {
        let r = SecretRef::new("nope");
        assert!(matches!(
            r.resolve(&source()),
            Err(SecretError::NotFound(p)) if p == "nope"
        ));
    }

    #[test]
    fn redaction_never_prints_the_value() {
        let r = SecretRef::new("db");
        let value = r.resolve(&source()).unwrap();
        assert_eq!(redacted_string(&value), "[redacted]");
        assert_eq!(format!("{:?}", Redacted(&value)), "[redacted]");
    }

    #[test]
    fn ref_serialises_as_indirection_only() {
        let json = serde_json::to_string(&SecretRef::new("db")).unwrap();
        assert_eq!(json, r#"{"secret_ref":"db"}"#);
    }
}
