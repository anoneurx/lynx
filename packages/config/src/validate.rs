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

    // ── §Z Step 18: answer-layer contracts (any environment) ─────────────────
    // The answer layer only exists with these settings; a config that turns
    // them off configures a different product (docs/ai/astra.md).
    if config.ai.enabled {
        if config.ai.provider == "none" {
            issues.push(ValidationIssue::new(
                "ai.provider",
                "ai.enabled requires a concrete provider",
            ));
        }
        if !config.ai.require_citations {
            issues.push(ValidationIssue::new(
                "ai.require_citations",
                "citation verification is a non-negotiable contract of the \\
                 answer layer (docs/ai/astra.md)",
            ));
        }
        if !config.ai.fail_closed {
            issues.push(ValidationIssue::new(
                "ai.fail_closed",
                "fail-closed (no verified citations → no answer) is a \
                 non-negotiable contract of the answer layer (docs/ai/astra.md)",
            ));
        }
        if config
            .ai
            .service_bind
            .parse::<std::net::SocketAddr>()
            .is_err()
        {
            issues.push(ValidationIssue::new(
                "ai.service_bind",
                "must be a socket address like 127.0.0.1:8180 (the answer process binds this)",
            ));
        }
    }

    // The public API dials the answer plane when configured; a malformed dial
    // address must fail at boot, not on the first search.
    if !config.ai.answer_service.is_empty()
        && !config.ai.answer_service.starts_with("http://")
        && !config.ai.answer_service.starts_with("https://")
    {
        issues.push(ValidationIssue::new(
            "ai.answer_service",
            "must be an http(s) base URL like https://ai.internal:8443/, or empty \
             to keep answers off",
        ));
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
        // §Z Step 20: the serving index is state, not scratch space — a /tmp generation
        // is wiped on reboot and the API boots refusing to serve (apps/api/src/reader.rs).
        if config.index.dir.trim().is_empty() {
            issues.push(ValidationIssue::new(
                "index.dir",
                "production requires an on-disk index directory",
            ));
        }
        if config.index.dir.starts_with("/tmp") {
            issues.push(ValidationIssue::new(
                "index.dir",
                "production refuses a /tmp index: the generation is wiped on reboot and \
                 the API would refuse to serve; use a state path like /var/lib/lynx/index",
            ));
        }
        if config.ai.enabled {
            if config.ai.model.trim().is_empty() {
                issues.push(ValidationIssue::new(
                    "ai.model",
                    "ai.enabled requires a pinned model id in production",
                ));
            }
            // The endpoint a production process confides its corpus snippets
            // to must not leak them in plain text over the network.
            if !config.ai.endpoint.starts_with("https://") {
                issues.push(ValidationIssue::new(
                    "ai.endpoint",
                    "ai endpoint must be https in production: the queries and \
                     source snippets it carries are corpus-grade content",
                ));
            }
            // The same rule for the search plane dialing the answer plane: the
            // request carries the active query, which is corpus-grade content.
            if !config.ai.answer_service.is_empty()
                && !config.ai.answer_service.starts_with("https://")
            {
                issues.push(ValidationIssue::new(
                    "ai.answer_service",
                    "answer_service must be https in production: the request \
                     carries the active query text",
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
        // §Z Step 17 production privacy check: the no-third-party-request invariant
        // (spec §N) is a policy the website itself advertises in the CSP
        // (`default-src 'none'`), so the config cannot quietly loosen it.
        if config.privacy.third_party_assets != "forbidden" {
            issues.push(ValidationIssue::new(
                "privacy.third_party_assets",
                "production refuses third-party assets; the served CSP allows none, \
                 so a config that would load them can never work (PRIVACY.md §8)",
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
    fn production_refuses_any_third_party_asset_policy_short_of_forbidden() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.instance_id = "prod-1".into();
        c.privacy.third_party_assets = "self-hosted-only".into();
        let issues = validate(&c).unwrap_err();
        assert!(issues
            .iter()
            .any(|i| i.field == "privacy.third_party_assets"));

        // The shipped production file keeps the invariant (`"forbidden"`).
        let prod = Config::from_toml_layers(&[
            include_str!("../../../config/example.toml"),
            include_str!("../../../config/production.toml"),
        ])
        .unwrap();
        assert_eq!(prod.privacy.third_party_assets, "forbidden");
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
    fn production_requires_an_https_ai_endpoint() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.instance_id = "prod-1".into();
        c.ai.enabled = true;
        c.ai.provider = "openai-compatible".into();
        c.ai.model = "m-1".into();
        c.ai.endpoint = "http://ai.internal:8080/v1".into();
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ai.endpoint"));

        let mut ok = c;
        ok.ai.endpoint = "https://ai.internal:8443/v1".into();
        let faults = validate(&ok).unwrap_err();
        assert!(
            !faults.iter().any(|i| i.field == "ai.endpoint"),
            "https endpoint accepted: {faults:?}"
        );
    }

    #[test]
    fn disabling_citation_verification_or_fail_closed_never_validates() {
        for field in ["require_citations", "fail_closed"] {
            let mut c = dev();
            c.ai.enabled = true;
            c.ai.provider = "openai-compatible".into();
            c.ai.model = "m-1".into();
            c.ai.endpoint = "https://ai.test/v1".into();
            match field {
                "require_citations" => c.ai.require_citations = false,
                "fail_closed" => c.ai.fail_closed = false,
                _ => unreachable!(),
            }
            let issues = validate(&c).unwrap_err();
            assert!(
                issues.iter().any(|i| { i.field == format!("ai.{field}") }),
                "{field} is non-negotiable: docs/ai/astra.md"
            );
        }
    }

    #[test]
    fn an_ai_provider_named_none_cannot_back_an_enabled_plane() {
        let mut c = dev();
        c.ai.enabled = true;
        c.ai.provider = "none".into();
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ai.provider"));
    }

    #[test]
    fn a_malformed_answer_service_never_validates() {
        let mut c = dev();
        c.ai.answer_service = "127.0.0.1:8180".into(); // missing scheme
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ai.answer_service"));

        let mut ok = c;
        ok.ai.answer_service = "http://127.0.0.1:8180".into();
        assert!(
            validate(&ok).is_ok(),
            "a well-formed dev dial address is accepted: {:?}",
            validate(&ok)
        );
    }

    #[test]
    fn production_dials_the_answer_plane_over_https_only() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.instance_id = "prod-1".into();
        c.ai.enabled = true;
        c.ai.provider = "openai-compatible".into();
        c.ai.model = "m-1".into();
        c.ai.endpoint = "https://ai-internal:8443/v1".into();
        c.ai.answer_service = "http://ai-internal:8180".into();
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ai.answer_service"));

        let mut ok = c;
        ok.ai.answer_service = "https://ai-internal:8443".into();
        let faults = validate(&ok).unwrap_err();
        assert!(
            !faults.iter().any(|i| i.field == "ai.answer_service"),
            "https dial address is accepted in production: {faults:?}"
        );
    }

    #[test]
    fn an_enabled_answer_plane_needs_a_parsable_bind() {
        let mut c = dev();
        c.ai.enabled = true;
        c.ai.provider = "openai-compatible".into();
        c.ai.model = "m-1".into();
        c.ai.service_bind = "not-an-address".into();
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ai.service_bind"));
    }

    #[test]
    fn production_refuses_a_tmp_serving_index() {
        let mut c = dev();
        c.core.environment = Environment::Production;
        c.core.instance_id = "prod-1".into();
        c.index.dir = "/tmp/lynx-index".into();
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "index.dir"), "{issues:?}");

        let mut ok = c;
        ok.index.dir = "/var/lib/lynx/index".into();
        let faults = validate(&ok).unwrap_err();
        assert!(
            !faults.iter().any(|i| i.field == "index.dir"),
            "a state path is accepted: {faults:?}"
        );

        let mut blank = dev();
        blank.core.environment = Environment::Production;
        blank.core.instance_id = "prod-1".into();
        blank.index.dir = "   ".into();
        let issues = validate(&blank).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "index.dir"));
    }

    #[test]
    fn out_of_range_weight_fails() {
        let mut c = dev();
        c.ranking.signals.bm25 = 1.5;
        let issues = validate(&c).unwrap_err();
        assert!(issues.iter().any(|i| i.field == "ranking.signals.bm25"));
    }
}
