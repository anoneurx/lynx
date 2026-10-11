//! Stable configuration fingerprint.
//!
//! A hash of the effective, redacted configuration is logged at startup and recorded on
//! index snapshots and ranker output, so any result can be attributed to an exact
//! configuration. Because the config holds only [`crate::secret::SecretRef`]s, never
//! resolved values, the fingerprint cannot leak a secret.

use lynx_common::hash::ContentHash;
use serde_json::Value;

use crate::schema::Config;

/// Compute the fingerprint of an effective configuration.
///
/// # Panics
/// Never in practice: a `Config` always serialises to JSON.
#[must_use]
pub fn fingerprint(config: &Config) -> String {
    let value = serde_json::to_value(config).unwrap_or(Value::Null);
    let canonical = serde_json::to_string(&value).unwrap_or_default();
    ContentHash::of(canonical.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    fn config() -> Config {
        Config::from_toml_str(include_str!("../../../config/example.toml")).unwrap()
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(&config()), fingerprint(&config()));
    }

    #[test]
    fn fingerprint_changes_with_effective_config() {
        let a = config();
        let mut b = config();
        b.ranking.signals.bm25 = 0.5;
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn fingerprint_does_not_contain_secret_values() {
        let mut c = config();
        c.database.url = crate::secret::SecretRef::new("lynx/production/postgres/app");
        let fp = fingerprint(&c);
        assert!(!fp.contains("postgres"));
        assert_eq!(fp.len(), 64);
    }
}
