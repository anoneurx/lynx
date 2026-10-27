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
    // Observability (§Z Step 16): sampling ratio and retention are privacy controls,
    // not tuning knobs — a full-sample trace contains URLs from the request line.
    if !(0.0..=1.0).contains(&config.observability.tracing_sample_ratio) {
        issues.push(ValidationIssue::new(
            "observability.tracing_sample_ratio",
            "must be within [0, 1]",
        ));
    }
    if config.observability.tracing && config.observability.tracing_endpoint.trim().is_empty() {
        issues.push(ValidationIssue::new(
            "observability.tracing_endpoint",
            "tracing enabled without a collector endpoint ships spans nowhere",
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
        if config.observability.log_retention_days > 14 {
            issues.push(ValidationIssue::new(
                "observability.log_retention_days",
                "logs are dropped after 14 days: there is nothing in them worth \
                 archiving that is not in Postgres, and longer retention is a privacy \
                 liability (docs/operations/monitoring.md)",
            ));
        }
        if config.observability.tracing_sample_ratio > 0.01 {
            issues.push(ValidationIssue::new(
                "observability.tracing_sample_ratio",
                "production head-sampling must stay at or below 1% — traces carry the \
                 request line, and high-rate traces are a privacy leak with a short \
                 time constant",
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
    fn sampling_ratio_must_be_a_probability() {
        let mut c = dev();
        c.observability.tracing_sample_ratio = 1.5;
        let issues = validate(&c).unwrap_err();
        assert!(issues
            .iter()
            .any(|i| i.field == "observability.tracing_sample_ratio"));
    }

    #[test]
    fn production_caps_traces_at_one_percent_and_logs_at_fourteen_days() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.instance_id = "prod-1".into();
        c.observability.tracing_sample_ratio = 0.50;
        c.observability.log_retention_days = 30;
        let issues = validate(&c).unwrap_err();
        let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
        assert!(fields.contains(&"observability.tracing_sample_ratio"));
        assert!(fields.contains(&"observability.log_retention_days"));

        // The shipping production file is inside both caps (parsed with the example
        // layer, the way the runtime loads environment files).
        let prod = Config::from_toml_layers(&[
            include_str!("../../../config/example.toml"),
            include_str!("../../../config/production.toml"),
        ])
        .unwrap();
        assert!(validate(&prod).is_ok(), "{:?}", validate(&prod));
    }

    #[test]
    fn tracing_enabled_without_a_collector_is_a_misconfiguration() {
        let mut c = dev();
        c.observability.tracing = true;
        c.observability.tracing_endpoint = String::new();
        let issues = validate(&c).unwrap_err();
        assert!(issues
            .iter()
            .any(|i| i.field == "observability.tracing_endpoint"));
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
