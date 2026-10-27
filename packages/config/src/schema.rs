//! Typed configuration schema.
//!
//! One module-of-structs per category, mirroring `config/*.toml`. Every struct is
//! `deny_unknown_fields`: a typo'd key is a hard error, never a silent default.
//!
//! Secret *values* never appear here. A place that needs a secret holds a
//! [`crate::secret::SecretRef`] and resolves it at startup.

use serde::{Deserialize, Serialize};

use crate::secret::SecretRef;

/// Top-level configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Process-level settings.
    pub core: Core,
    /// Public API settings.
    #[serde(default)]
    pub api: Api,
    /// Database connection settings.
    #[serde(default)]
    pub database: Database,
    /// Cache namespaces.
    #[serde(default)]
    pub cache: Cache,
    /// Index settings.
    #[serde(default)]
    pub index: Index,
    /// Ranking settings.
    #[serde(default)]
    pub ranking: Ranking,
    /// Crawler settings.
    #[serde(default)]
    pub crawler: Crawler,
    /// Privacy settings (weakening any requires an ADR).
    #[serde(default)]
    pub privacy: Privacy,
    /// Security settings.
    #[serde(default)]
    pub security: Security,
    /// Optional AI answer layer.
    #[serde(default)]
    pub ai: Ai,
    /// Observability settings.
    #[serde(default)]
    pub observability: Observability,
    /// Admin surface settings.
    #[serde(default)]
    pub admin: Admin,
}

/// Deployment environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    /// Local development.
    #[default]
    Development,
    /// Staging, production-like.
    Staging,
    /// Production.
    Production,
}

impl Environment {
    /// Whether this is the production environment.
    #[must_use]
    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// Process-level settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Core {
    /// Deployment environment.
    #[serde(default)]
    pub environment: Environment,
    /// Log filter level.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Log output format.
    #[serde(default)]
    pub log_format: LogFormat,
    /// Instance identity for logs.
    #[serde(default)]
    pub instance_id: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Structured JSON (production).
    #[default]
    Json,
    /// Human-friendly (development only).
    Pretty,
}

/// Public API settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Api {
    /// Listen address.
    pub bind: String,
    /// Public origin.
    pub public_url: String,
    /// Allowed CORS origins. Never `*` in production.
    pub cors_origins: Vec<String>,
    /// Number of trusted proxy hops in front of us.
    pub trusted_proxy_depth: u8,
    /// Maximum accepted query length in bytes.
    pub max_query_bytes: usize,
    /// Maximum results a client may request per page.
    pub max_results_per_page: usize,
    /// Default results per page.
    pub default_results: usize,
    /// Candidate set size pulled before ranking.
    pub max_candidates: usize,
    /// Server-side request budget.
    pub request_timeout_ms: u64,
    /// Pagination configuration.
    #[serde(default)]
    pub pagination: Pagination,
}

/// Pagination configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pagination {
    /// Pagination mode.
    pub mode: PaginationMode,
    /// Maximum page size.
    pub max_limit: usize,
}

/// Pagination mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaginationMode {
    /// Opaque cursor, stable under a moving index.
    Cursor,
    /// Offset page numbers.
    Page,
}

/// Database settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    /// Connection URL, by reference.
    pub url: SecretRef,
    /// Maximum pool connections.
    pub max_connections: u32,
    /// Minimum pool connections.
    pub min_connections: u32,
    /// Server-side statement timeout.
    pub statement_timeout_ms: u64,
    /// Pool acquire timeout.
    pub acquire_timeout_ms: u64,
}

/// Cache namespaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    /// Query-response cache.
    pub query: QueryCache,
    /// Index/segment cache.
    pub index: IndexCache,
}

/// Query-response cache settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryCache {
    /// Whether enabled.
    pub enabled: bool,
    /// Entry TTL.
    pub ttl_seconds: u64,
    /// Maximum entries.
    pub max_entries: usize,
    /// Keying secret, by reference.
    pub key_secret: SecretRef,
}

/// Index cache settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexCache {
    /// Whether enabled.
    pub enabled: bool,
    /// Entry TTL.
    pub ttl_seconds: u64,
}

/// Index settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    /// On-disk index directory.
    pub dir: String,
    /// Shard count.
    pub shards: u32,
    /// Memory budget in MiB.
    pub memory_budget_mb: u64,
    /// Commit interval.
    pub commit_interval_seconds: u64,
    /// Commit after this many docs.
    pub commit_docs_threshold: u64,
    /// Search threads.
    pub num_search_threads: usize,
    /// Merge policy segment target.
    pub merge_policy_segments: usize,
}

/// Ranking settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ranking {
    /// Ranking config version; bump on any weight change.
    pub config_version: String,
    /// Candidate set size.
    pub candidates: usize,
    /// Snippet length cap in characters.
    pub snippet_max_chars: usize,
    /// Signal weights.
    pub signals: Signals,
}

/// Ranking signal weights. The transparent core of ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signals {
    /// Lexical relevance weight.
    pub bm25: f64,
    /// Recency weight.
    pub freshness: f64,
    /// Static quality weight.
    pub quality: f64,
    /// Link-graph authority weight.
    pub authority: f64,
    /// Context match weight.
    pub context: f64,
    /// Spam penalty, applied multiplicatively.
    pub spam_penalty: f64,
}

/// Crawler settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crawler {
    /// User-agent string.
    pub user_agent: String,
    /// robots.txt user-agent group.
    pub user_agent_group: String,
    /// Global in-flight fetches per worker.
    pub concurrency: usize,
    /// Allowed URL schemes.
    pub allowed_schemes: Vec<String>,
    /// Allowed ports.
    pub allowed_ports: Vec<u16>,
    /// Maximum redirects.
    pub max_redirects: u8,
    /// Maximum crawl depth.
    pub max_depth: u16,
    /// Maximum response body bytes.
    pub max_response_bytes: u64,
    /// Maximum decompression ratio.
    pub max_decompression_ratio: u32,
    /// Connect timeout.
    pub connect_timeout_ms: u64,
    /// Header timeout.
    pub header_timeout_ms: u64,
    /// Read-idle timeout.
    pub read_idle_timeout_ms: u64,
    /// Total request timeout.
    pub total_timeout_ms: u64,
    /// Whether robots.txt is respected. Disabling blocks production start.
    pub respect_robots: bool,
    /// Whether robots.txt responses are cached.
    pub cache_robots: bool,
    /// Per-host budget.
    pub budget: CrawlBudget,
}

/// Per-host crawl budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrawlBudget {
    /// Maximum pages per host per day.
    pub max_pages_per_host_day: u64,
    /// Maximum bytes per host per day.
    pub max_bytes_per_host_day: u64,
    /// Floor on inter-request delay.
    pub min_inter_request_ms: u64,
    /// Maximum concurrent fetches per host.
    pub max_concurrent_per_host: usize,
}

/// Privacy settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    /// Whether queries are logged. MUST be false everywhere.
    pub query_logging: bool,
    /// Whether query *shapes* feed metrics.
    pub metrics_query_shapes: bool,
    /// Whether responses are cached.
    pub response_cache: bool,
    /// Third-party asset policy.
    pub third_party_assets: String,
    /// Rate-limit settings.
    #[serde(default)]
    pub rate_limit: RateLimit,
}

/// Rate-limit settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    /// Whether enabled.
    pub enabled: bool,
    /// IPv4 prefix bits.
    pub prefix_bits_v4: u8,
    /// IPv6 prefix bits.
    pub prefix_bits_v6: u8,
    /// Salt rotation period.
    pub salt_ttl_hours: u64,
    /// Salt, by reference.
    pub salt: SecretRef,
    /// Anonymous requests per minute.
    pub anonymous_per_minute: u32,
    /// Authenticated requests per minute.
    pub developer_per_minute: u32,
}

/// Security settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Security {
    /// Minimum TLS version.
    pub tls_min_version: String,
    /// Allowed cipher suites.
    pub allowed_ciphers: Vec<String>,
    /// HSTS max age in seconds.
    pub hsts_max_age: u64,
    /// Whether the admin surface requires VPN access.
    pub admin_require_vpn: bool,
    /// API key prefix.
    pub api_key_prefix: String,
}

/// AI answer-layer settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ai {
    /// Whether enabled.
    pub enabled: bool,
    /// Provider kind.
    pub provider: String,
    /// Pinned model id (required when enabled in production).
    pub model: String,
    /// Provider endpoint.
    pub endpoint: String,
    /// Provider key, by reference.
    pub api_key: SecretRef,
    /// Maximum source tokens.
    pub max_source_tokens: u32,
    /// Maximum sources.
    pub max_sources: u32,
    /// Answer cache TTL.
    pub answer_cache_ttl: u64,
    /// Daily token budget.
    pub daily_token_budget: u64,
    /// Whether citations are required.
    pub require_citations: bool,
    /// Whether to fail closed when citations cannot be verified.
    pub fail_closed: bool,
    /// Where the answer-plane process listens. The lynx-ai binary binds this; the
    /// public API dials it through `answer_service`. Two views of one process:
    /// the bind is local, the dial address may go through a service DNS name.
    #[serde(default = "default_ai_service_bind")]
    pub service_bind: String,
    /// The public API's view of the answer plane (scheme + host + port, e.g.
    /// `http://127.0.0.1:8180`). Empty disables query→answer merging in the API:
    /// SERP only, and the answer plane is not contacted.
    #[serde(default)]
    pub answer_service: String,
    /// Per-request timeout for the answer call, in milliseconds. A timed-out answer
    /// never fails the search: the SERP renders without it.
    #[serde(default = "default_answer_timeout_ms")]
    pub answer_timeout_ms: u64,
    /// Trust gate for the answer context: a page whose spam score leaves less
    /// remaining trust than this (`1.0 - spam_score`) is withheld from the head
    /// the answer plane may ground from. `0.0` disables the gate.
    #[serde(default)]
    pub min_source_trust: f64,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "none".to_string(),
            model: String::new(),
            endpoint: String::new(),
            api_key: SecretRef::new("lynx/development/ai/provider"),
            max_source_tokens: 6000,
            max_sources: 10,
            answer_cache_ttl: 600,
            daily_token_budget: 2_000_000,
            require_citations: true,
            fail_closed: true,
            service_bind: default_ai_service_bind(),
            answer_service: String::new(),
            answer_timeout_ms: default_answer_timeout_ms(),
            min_source_trust: 0.0,
        }
    }
}

fn default_ai_service_bind() -> String {
    "127.0.0.1:8180".to_string()
}

fn default_answer_timeout_ms() -> u64 {
    1500
}

/// Observability settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observability {
    /// Metrics bind address.
    pub metrics_bind: String,
    /// Metrics path.
    pub metrics_path: String,
    /// Whether distributed tracing is on.
    pub tracing: bool,
    /// Tracing collector endpoint.
    pub tracing_endpoint: String,
    /// Tracing sample ratio.
    pub tracing_sample_ratio: f64,
    /// Log retention.
    pub log_retention_days: u64,
}

/// Admin surface settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admin {
    /// Whether basic auth is enabled (development only).
    pub enable_basic_auth: bool,
    /// Admin bind address.
    pub bind: String,
}

// Defaults mirror `config/example.toml`. Where a value is a secret, the default is a
// reference placeholder; resolution fails loudly if it is not provided.

impl Default for Api {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".into(),
            public_url: "http://localhost:3000".into(),
            cors_origins: vec!["http://localhost:3000".into()],
            trusted_proxy_depth: 0,
            max_query_bytes: 512,
            max_results_per_page: 50,
            default_results: 10,
            max_candidates: 2000,
            request_timeout_ms: 3000,
            pagination: Pagination::default(),
        }
    }
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            mode: PaginationMode::Cursor,
            max_limit: 50,
        }
    }
}

impl Default for Database {
    fn default() -> Self {
        Self {
            url: SecretRef::new("lynx/development/postgres/app"),
            max_connections: 20,
            min_connections: 2,
            statement_timeout_ms: 30_000,
            acquire_timeout_ms: 5_000,
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            query: QueryCache {
                enabled: true,
                ttl_seconds: 120,
                max_entries: 50_000,
                key_secret: SecretRef::new("lynx/development/cache/query"),
            },
            index: IndexCache {
                enabled: true,
                ttl_seconds: 600,
            },
        }
    }
}

impl Default for Index {
    fn default() -> Self {
        Self {
            dir: "/tmp/lynx-dev-index".into(),
            shards: 1,
            memory_budget_mb: 1024,
            commit_interval_seconds: 20,
            commit_docs_threshold: 5000,
            num_search_threads: 4,
            merge_policy_segments: 8,
        }
    }
}

impl Default for Ranking {
    fn default() -> Self {
        Self {
            config_version: "v0.1.0".into(),
            candidates: 2000,
            snippet_max_chars: 240,
            signals: Signals::default(),
        }
    }
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            bm25: 1.00,
            freshness: 0.18,
            quality: 0.12,
            authority: 0.15,
            context: 0.10,
            spam_penalty: 0.40,
        }
    }
}

impl Default for Crawler {
    fn default() -> Self {
        Self {
            user_agent: "LynxBot/0.1 (+https://anoneurx.com/lynx/bot)".into(),
            user_agent_group: "LynxBot".into(),
            concurrency: 8,
            allowed_schemes: vec!["http".into(), "https".into()],
            allowed_ports: vec![80, 443],
            max_redirects: 5,
            max_depth: 20,
            max_response_bytes: 5 * 1024 * 1024,
            max_decompression_ratio: 100,
            connect_timeout_ms: 5_000,
            header_timeout_ms: 5_000,
            read_idle_timeout_ms: 5_000,
            total_timeout_ms: 15_000,
            respect_robots: true,
            cache_robots: true,
            budget: CrawlBudget::default(),
        }
    }
}

impl Default for CrawlBudget {
    fn default() -> Self {
        Self {
            max_pages_per_host_day: 5000,
            max_bytes_per_host_day: 5 * 1024 * 1024 * 1024,
            min_inter_request_ms: 1000,
            max_concurrent_per_host: 2,
        }
    }
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            query_logging: false,
            metrics_query_shapes: true,
            response_cache: true,
            third_party_assets: "forbidden".into(),
            rate_limit: RateLimit::default(),
        }
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            enabled: true,
            prefix_bits_v4: 24,
            prefix_bits_v6: 48,
            salt_ttl_hours: 24,
            salt: SecretRef::new("lynx/development/ratelimit/salt"),
            anonymous_per_minute: 30,
            developer_per_minute: 300,
        }
    }
}

impl Default for Security {
    fn default() -> Self {
        Self {
            tls_min_version: "1.3".into(),
            allowed_ciphers: vec![
                "TLS_AES_256_GCM_SHA384".into(),
                "TLS_CHACHA20_POLY1305_SHA256".into(),
            ],
            hsts_max_age: 63_072_000,
            admin_require_vpn: true,
            api_key_prefix: "lnx_".into(),
        }
    }
}

impl Default for Observability {
    fn default() -> Self {
        Self {
            metrics_bind: "0.0.0.0:9090".into(),
            metrics_path: "/metrics".into(),
            tracing: false,
            tracing_endpoint: String::new(),
            tracing_sample_ratio: 0.01,
            log_retention_days: 14,
        }
    }
}

impl Default for Admin {
    fn default() -> Self {
        Self {
            enable_basic_auth: true,
            bind: "127.0.0.1:9091".into(),
        }
    }
}
