//! Row types mirroring the physical schema.
//!
//! Each row derives `sqlx::FromRow` and maps to a `lynx_models` domain type where one
//! exists. Field names match the columns in `database/migrations`, so the SELECT
//! lists in [`crate::repos`] can return these directly.

use lynx_models::domain::{Domain, DomainId, Host, HostId, Url, UrlId};
use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Lifecycle of a frontier entry. Matches the Postgres `crawl_state` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "crawl_state", rename_all = "lowercase")]
pub enum CrawlState {
    /// Discovered, not yet claimable.
    Pending,
    /// Claimable.
    Ready,
    /// Held by a worker.
    Claimed,
    /// Fetched and processed.
    Done,
    /// Refused by robots or policy.
    Denied,
    /// Excluded by a ban.
    Banned,
}

/// How a frontier entry was discovered. Matches `discovery_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "discovery_source", rename_all = "lowercase")]
pub enum DiscoverySource {
    /// Found on a crawled page.
    Outlink,
    /// Found in a sitemap.
    Sitemap,
    /// Found in an RSS/Atom feed.
    Feed,
    /// A seed URL.
    Seed,
    /// Submitted by an operator.
    Operator,
}

/// Document lifecycle. Matches the Postgres `document_state` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "document_state", rename_all = "snake_case")]
pub enum DocumentState {
    /// Indexed and eligible.
    Active,
    /// Present in the index but excluded from results.
    ShadowSuppressed,
    /// Indexed as a 404.
    #[sqlx(rename = "soft_404")]
    Soft404,
    /// Removed; must never be resurrectable.
    Deleted,
}

/// A registrable domain.
#[derive(Debug, Clone, FromRow)]
pub struct DomainRow {
    /// `domain.id`.
    pub id: Uuid,
    /// Registrable domain, lowercased punycode.
    pub name: String,
    /// First observation.
    pub first_seen: OffsetDateTime,
    /// Last update.
    pub updated_at: OffsetDateTime,
}

/// A specific hostname.
#[derive(Debug, Clone, FromRow)]
pub struct HostRow {
    /// `host.id`.
    pub id: Uuid,
    /// Owning registrable domain.
    pub domain_id: Uuid,
    /// Full hostname, lowercased punycode.
    pub name: String,
    /// Minimum seconds between requests.
    pub rate_limit_seconds: i32,
    /// Requests in the current window.
    pub requests_in_window: i32,
    /// Earliest next request.
    pub next_request_at: OffsetDateTime,
    /// Per-host in-flight cap.
    pub concurrency_limit: i32,
    /// Consecutive failures observed.
    pub consecutive_failures: i32,
    /// First observation.
    pub first_seen: OffsetDateTime,
    /// Last update.
    pub updated_at: OffsetDateTime,
}

/// A canonical URL.
#[derive(Debug, Clone, FromRow)]
pub struct UrlRow {
    /// `url.id`.
    pub id: Uuid,
    /// `sha256(normalised_url)`.
    pub url_hash: Vec<u8>,
    /// Scheme.
    pub scheme: String,
    /// Owning host.
    pub host_id: Uuid,
    /// Path.
    pub path: String,
    /// Query string, if any.
    pub query_string: Option<String>,
    /// Fragment, if any (stored but never crawled).
    pub fragment: Option<String>,
    /// Canonicalised URL string.
    pub canonical_url: String,
    /// What canonicalisation changed.
    pub normalised: Value,
    /// First observation.
    pub first_seen: OffsetDateTime,
    /// Last observation.
    pub last_seen: OffsetDateTime,
}

/// A document label row.
#[derive(Debug, Clone, FromRow)]
pub struct DocumentRow {
    /// Document id (UUID v5 from the canonical URL).
    pub doc_id: Uuid,
    /// Owning URL.
    pub url_id: Uuid,
    /// Lifecycle state.
    pub state: DocumentState,
    /// `SipHash` of the extracted text.
    pub content_hash: Option<Vec<u8>>,
    /// Dedup cluster membership.
    pub cluster_id: Option<Uuid>,
    /// Detected language.
    pub lang: Option<String>,
    /// Publication time.
    pub published_at: Option<OffsetDateTime>,
    /// Modification time.
    pub modified_at: Option<OffsetDateTime>,
    /// Declared content type.
    pub content_type: Option<String>,
    /// Quality label.
    pub quality_score: Option<f32>,
    /// Spam label.
    pub spam_score: Option<f32>,
    /// Generation currently serving this document.
    pub indexed_generation: Option<i64>,
    /// First time indexed.
    pub first_indexed_at: Option<OffsetDateTime>,
    /// Last content change.
    pub last_changed_at: OffsetDateTime,
    /// Row creation.
    pub created_at: OffsetDateTime,
    /// Row update.
    pub updated_at: OffsetDateTime,
}

/// A frontier queue entry.
#[derive(Debug, Clone, FromRow)]
pub struct CrawlFrontierRow {
    /// Entry id.
    pub id: Uuid,
    /// Target URL (logical reference).
    pub url_id: Uuid,
    /// Target host (logical reference).
    pub host_id: Uuid,
    /// Higher is claimed first.
    pub priority: f32,
    /// Link depth.
    pub depth: i16,
    /// Queue state.
    pub state: CrawlState,
    /// Earliest claim time.
    pub not_before: OffsetDateTime,
    /// Attempts so far.
    pub attempts: i16,
    /// Target index generation.
    pub generation: Option<i64>,
    /// How it was discovered.
    pub discovery: DiscoverySource,
    /// Partition key.
    pub created_at: OffsetDateTime,
}

/// One crawl attempt.
#[derive(Debug, Clone, FromRow)]
pub struct CrawlAttemptRow {
    /// Attempt id.
    pub id: i64,
    /// URL attempted.
    pub url_id: Uuid,
    /// Host attempted.
    pub host_id: Uuid,
    /// Start time.
    pub started_at: OffsetDateTime,
    /// Finish time, if it finished.
    pub finished_at: Option<OffsetDateTime>,
    /// HTTP status, if a response was received.
    pub status_code: Option<i32>,
    /// Response content type.
    pub content_type: Option<String>,
    /// Bytes received.
    pub bytes: Option<i32>,
    /// Duration in milliseconds.
    pub duration_ms: Option<i32>,
    /// `ok` | `error` | `blocked` | `skipped`.
    pub outcome: String,
    /// Classified error, if any.
    pub error_class: Option<String>,
}

/// A link-graph edge.
#[derive(Debug, Clone, FromRow)]
pub struct LinkRow {
    /// Edge id.
    pub id: i64,
    /// Source document (logical reference).
    pub source_doc_id: Uuid,
    /// Target URL.
    pub target_url_id: Uuid,
    /// Target document, once indexed.
    pub target_doc_id: Option<Uuid>,
    /// Anchor text.
    pub anchor_text: Option<String>,
    /// Whether the link is `nofollow`.
    pub is_nofollow: bool,
    /// Whether the link is sponsored.
    pub is_sponsored: bool,
    /// `body` | `nav` | `footer` | `main`.
    pub context: Option<String>,
}

/// A host's robots policy, cached.
#[derive(Debug, Clone, FromRow)]
pub struct RobotsPolicyRow {
    /// Host the policy belongs to.
    pub host_id: Uuid,
    /// Raw robots.txt, retained for compliance disputes.
    pub raw: Option<String>,
    /// Parsed rules, sitemaps, and crawl-delay.
    pub parsed: Value,
    /// HTTP status of the fetch.
    pub status: Option<i32>,
    /// `ETag`, for conditional re-fetch.
    pub etag: Option<String>,
    /// Last-Modified, for conditional re-fetch.
    pub last_modified: Option<String>,
    /// When it was fetched.
    pub fetched_at: OffsetDateTime,
    /// When it expires.
    pub expires_at: OffsetDateTime,
    /// Parse error, if any.
    pub parse_error: Option<String>,
}

/// An index generation manifest.
#[derive(Debug, Clone, FromRow)]
pub struct IndexManifestRow {
    /// Generation number.
    pub generation: i64,
    /// Number of shards.
    pub shard_count: i32,
    /// Document count.
    pub doc_count: i64,
    /// Index schema version.
    pub schema_version: String,
    /// Tokenizer version.
    pub tokenizer_version: String,
    /// Ranking configuration version.
    pub ranking_config_version: String,
    /// Directory of the sealed generation.
    pub path: String,
    /// Whether this is the served generation.
    pub is_current: bool,
    /// When it was built.
    pub built_at: OffsetDateTime,
}

impl DomainRow {
    /// Convert to the shared domain type.
    #[must_use]
    pub fn to_domain(&self) -> Domain {
        Domain {
            id: DomainId::from_uuid(self.id),
            name: self.name.clone(),
        }
    }
}

impl HostRow {
    /// Convert to the shared host type.
    #[must_use]
    pub fn to_host(&self) -> Host {
        Host {
            id: HostId::from_uuid(self.id),
            domain_id: DomainId::from_uuid(self.domain_id),
            name: self.name.clone(),
        }
    }
}

impl UrlRow {
    /// Convert to the shared URL type.
    #[must_use]
    pub fn to_url(&self) -> Url {
        Url {
            id: UrlId::from_uuid(self.id),
            host_id: HostId::from_uuid(self.host_id),
            canonical: self.canonical_url.clone(),
            discovered_at: self.first_seen,
        }
    }
}
