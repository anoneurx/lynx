//! Environment-specific configuration validation.
//!
//! These rules exist because the failure they prevent is silent and dangerous: a
//! production process that logs queries, trusts a forged `X-Forwarded-For`, or runs an
//! unpinned model. Validation runs at startup and refuses to boot on any error.

use serde::{Deserialize, Serialize};

use crate::schema::{Config, Environment};

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// The dotted config path, e.g. `privacy.query_logging`.
    pub field: String,
    /// A human-readable explanation.
    pub message: String,
}

impl ValidationIssue {
    fn new(field: &str, message: &str) -> Self {
        Self {
            field: field.to_string(),
            message: message.to_string(),
        }
    }
}

/// Validate the effective configuration for its declared environment.
///
/// Returns every issue rather than the first, so a single run reports the full set.
///
/// # Errors
/// Returns the collected [`ValidationIssue`]s when any rule fails.
#[allow(clippy::too_many_lines)] // one flat pass over the rules is easier to audit
pub fn validate(config: &Config) -> Result<(), Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    let env = config.core.environment;
    let prod = env.is_production();

    // ── Rules that hold in every environment ────────────────────────────────────
    if config.privacy.query_logging {
        issues.push(ValidationIssue::new(
            "privacy.query_logging",
            "query logging is forbidden in every environment; see PRIVACY.md §8",
        ));
    }
    if config.api.max_query_bytes == 0 {
        issues.push(ValidationIssue::new(
            "api.max_query_bytes",
            "must be greater than zero",
        ));
    }
    if config.api.default_results > config.api.max_results_per_page {
        issues.push(ValidationIssue::new(
            "api.default_results",
            "must not exceed api.max_results_per_page",
        ));
    }
    if config.api.default_results == 0 {
        issues.push(ValidationIssue::new(
            "api.default_results",
            "must be greater than zero",
        ));
    }
    if config.api.trusted_proxy_depth > 5 {
        issues.push(ValidationIssue::new(
            "api.trusted_proxy_depth",
            "more than 5 trusted proxy hops is almost certainly a misconfiguration",
        ));
    }
    if config.crawler.allowed_ports.is_empty() {
        issues.push(ValidationIssue::new(
            "crawler.allowed_ports",
            "at least one port must be allowed",
        ));
    }
    let s = &config.ranking.signals;
    for (name, value) in [
        ("ranking.signals.bm25", s.bm25),
        ("ranking.signals.freshness", s.freshness),
        ("ranking.signals.quality", s.quality),
        ("ranking.signals.authority", s.authority),
        ("ranking.signals.context", s.context),
        ("ranking.signals.spam_penalty", s.spam_penalty),
    ] {
        if !(0.0..=1.0).contains(&value) {
            issues.push(ValidationIssue::new(name, "weight must be within [0, 1]"));
        }
    }

    // ── Production-only rules ───────────────────────────────────────────────────
    if prod {
        if matches!(config.core.log_format, crate::schema::LogFormat::Pretty) {
            issues.push(ValidationIssue::new(
                "core.log_format",
                "pretty logs are refused in production; use json",
            ));
        }
        if config.api.cors_origins.iter().any(|o| o == "*") {
            issues.push(ValidationIssue::new(
                "api.cors_origins",
                "wildcard CORS origins are forbidden in production",
            ));
        }
        if !config.crawler.respect_robots {
            issues.push(ValidationIssue::new(
                "crawler.respect_robots",
                "disabling robots respect blocks production start",
            ));
        }
        if !config.security.admin_require_vpn {
            issues.push(ValidationIssue::new(
                "security.admin_require_vpn",
                "the admin surface must require VPN access in production",
            ));
        }
        if config.admin.enable_basic_auth {
            issues.push(ValidationIssue::new(
                "admin.enable_basic_auth",
                "basic auth is refused in production; use SSO",
            ));
        }
        if config.security.tls_min_version != "1.3" {
            issues.push(ValidationIssue::new(
                "security.tls_min_version",
                "production requires TLS 1.3",
            ));
        }
        if config.ai.enabled {
            if config.ai.provider == "none" {
                issues.push(ValidationIssue::new(
                    "ai.provider",
                    "ai.enabled requires a concrete provider",
                ));
            }
            if config.ai.model.trim().is_empty() {
                issues.push(ValidationIssue::new(
                    "ai.model",
                    "ai.enabled requires a pinned model id in production",
                ));
            }
        }
        if config.core.instance_id.trim().is_empty() {
            issues.push(ValidationIssue::new(
                "core.instance_id",
                "production requires an instance id for log correlation",
            ));
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

/// The declared environment's name.
#[must_use]
pub fn environment_name(env: Environment) -> &'static str {
    match env {
        Environment::Development => "development",
        Environment::Staging => "staging",
        Environment::Production => "production",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    fn dev() -> Config {
        Config::from_toml_str(include_str!("../../../config/example.toml")).unwrap()
    }

    #[test]
    fn example_config_is_valid_for_its_environment() {
        assert!(validate(&dev()).is_ok(), "{:?}", validate(&dev()));
    }

    #[test]
    fn query_logging_always_fails() {
        let mut c = dev();
        c.privacy.query_logging = true;
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "privacy.query_logging"));
    }

    #[test]
    fn production_rejects_wildcard_cors_and_pretty_logs() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.log_format = crate::schema::LogFormat::Pretty;
        c.api.cors_origins = vec!["*".into()];
        c.core.instance_id = "prod-1".into();
        let issues = validate(&c).unwrap_err();
        let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
        assert!(fields.contains(&"api.cors_origins"));
        assert!(fields.contains(&"core.log_format"));
    }

    #[test]
    fn production_rejects_unpinned_ai_model() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.instance_id = "prod-1".into();
        c.ai.enabled = true;
        c.ai.provider = "openai-compatible".into();
        c.ai.model = String::new();
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ai.model"));
    }

    #[test]
    fn out_of_range_weight_fails() {
        let mut c = dev();
        c.ranking.signals.bm25 = 1.5;
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ranking.signals.bm25"));
    }
}
