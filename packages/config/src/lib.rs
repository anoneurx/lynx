//! `lynx-config` — layered configuration with strict validation and secret indirection.
//!
//! Precedence (lowest to highest):
//!
//! ```text
//! built-in defaults
//!   < config/{environment}.toml
//!     < config/local.toml         (gitignored developer overrides)
//!       < environment variables   (LYNX_*)
//!         < --config-override      (dev only; refused in production)
//!           < secret store          (values referenced by secret_ref only)
//! ```
//!
//! Layering is performed by deep-merging TOML documents, then deserialising once. The
//! final deserialisation is where `deny_unknown_fields` rejects a typo, in whichever
//! layer it appeared.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod fingerprint;
pub mod schema;
pub mod secret;
pub mod validate;

pub use schema::{Ai, Config, Environment};
pub use secret::{SecretError, SecretRef, SecretSource};

/// Configuration loading and validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("failed to read config: {0}")]
    Io(#[from] std::io::Error),
    /// The TOML was syntactically invalid or contained an unknown key.
    #[error("invalid config: {0}")]
    Toml(#[from] toml::de::Error),
    /// Validation failed.
    #[error("config validation failed: {}", format_issues(.0))]
    Validation(Vec<validate::ValidationIssue>),
}

fn format_issues(issues: &[validate::ValidationIssue]) -> String {
    issues
        .iter()
        .map(|i| format!("{}: {}", i.field, i.message))
        .collect::<Vec<_>>()
        .join("; ")
}

impl Config {
    /// Parse a single TOML document.
    ///
    /// # Errors
    /// Returns [`ConfigError::Toml`] for syntax errors, unknown keys, or missing
    /// required fields.
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(s)?)
    }

    /// Read and parse a single TOML file.
    ///
    /// # Errors
    /// Returns [`ConfigError::Io`] or [`ConfigError::Toml`].
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)?;
        Self::from_toml_str(&text)
    }

    /// Parse and deep-merge several TOML documents, later documents winning.
    ///
    /// # Errors
    /// Returns [`ConfigError::Toml`] if any document is invalid or the merged result
    /// contains unknown keys or misses a required field.
    pub fn from_toml_layers(layers: &[&str]) -> Result<Self, ConfigError> {
        let mut merged: Option<toml::Value> = None;
        for layer in layers {
            let value: toml::Value = toml::from_str(layer)?;
            match &mut merged {
                Some(base) => merge(base, value),
                None => merged = Some(value),
            }
        }
        let value = merged.unwrap_or(toml::Value::Table(toml::map::Map::new()));
        Ok(value.try_into()?)
    }

    /// Validate the effective configuration, returning every issue.
    ///
    /// # Errors
    /// Returns [`ConfigError::Validation`] listing all failed rules.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate::validate(self).map_err(ConfigError::Validation)
    }

    /// The stable fingerprint of this configuration (see [`fingerprint`]).
    #[must_use]
    pub fn fingerprint(&self) -> String {
        fingerprint::fingerprint(self)
    }

    /// A redacted JSON view, safe to log and to serve from the admin config endpoint.
    ///
    /// Because the schema stores only secret references, this view cannot contain a
    /// secret value by construction.
    ///
    /// # Panics
    /// Never in practice: a `Config` always serialises to JSON.
    #[must_use]
    pub fn redacted(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Validate and return `self`, for fluent construction.
    ///
    /// # Errors
    /// Returns [`ConfigError::Validation`].
    pub fn validated(self) -> Result<Self, ConfigError> {
        self.validate()?;
        Ok(self)
    }
}

/// Deep-merge `overlay` into `base`. Tables merge recursively; every other value is
/// replaced wholesale by the overlay.
fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge(base_value, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config/example.toml");

    #[test]
    #[allow(clippy::float_cmp)] // exact literals round-trip through TOML
    fn example_parses_and_validates() {
        let c = Config::from_toml_str(EXAMPLE).unwrap();
        c.validate().unwrap();
        assert_eq!(c.core.environment, Environment::Development);
        assert_eq!(c.ranking.signals.spam_penalty, 0.40);
    }

    #[test]
    fn unknown_key_is_a_hard_error() {
        let bad = "[core]\nenvironment = \"development\"\nlog_level = \"info\"\n\
                   log_format = \"json\"\ninstance_id = \"x\"\nnonsense = true\n";
        assert!(matches!(
            Config::from_toml_str(bad),
            Err(ConfigError::Toml(_))
        ));
    }

    #[test]
    fn later_layer_overrides_earlier() {
        let base = "[core]\nenvironment = \"development\"\n";
        let overlay = "[core]\nenvironment = \"staging\"\nlog_level = \"debug\"\n";
        let c = Config::from_toml_layers(&[base, overlay]).unwrap();
        assert_eq!(c.core.environment, Environment::Staging);
        assert_eq!(c.core.log_level, "debug");
        assert_eq!(c.api.bind, "0.0.0.0:8080", "unset keys keep defaults");
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact literals round-trip through TOML
    fn nested_tables_merge_rather_than_replace() {
        let base = "[core]\nenvironment = \"development\"\n\
                    [ranking]\nconfig_version = \"v1\"\ncandidates = 2000\nsnippet_max_chars = 240\n\
                    [ranking.signals]\nbm25 = 1.0\nfreshness = 0.18\nquality = 0.12\nauthority = 0.15\ncontext = 0.10\nspam_penalty = 0.40\n";
        let overlay = "[ranking.signals]\nbm25 = 0.9\n";
        let c = Config::from_toml_layers(&[base, overlay]).unwrap();
        assert_eq!(c.ranking.signals.bm25, 0.9);
        assert_eq!(c.ranking.signals.freshness, 0.18, "sibling signal kept");
    }

    #[test]
    fn redacted_view_has_no_secret_values() {
        let c = Config::from_toml_str(EXAMPLE).unwrap();
        let json = c.redacted();
        let s = json.to_string();
        assert!(!s.contains("password"));
        assert!(s.contains("secret_ref"));
    }
}
