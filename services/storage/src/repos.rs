//! Repository functions over the physical schema.
//!
//! These are thin, explicit queries rather than an ORM: the SQL is the contract and
//! the column lists are written out so a schema change is a reviewable diff here.
//! Every function takes a pool and returns row types from [`crate::rows`].
//!
//! Functions that mutate state are idempotent by construction where they can be —
//! the crawl pipeline may replay a step after a crash, and a replay must not create a
//! second row for the same URL.

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StorageError;
use crate::rows::{
    CrawlAttemptRow, CrawlFrontierRow, DiscoverySource, DocumentRow, DomainRow, HostRow, UrlRow,
};

/// Fields needed to insert or update a URL.
#[derive(Debug)]
pub struct NewUrl<'a> {
    /// Owning host.
    pub host_id: Uuid,
    /// `sha256(normalised_url)`.
    pub url_hash: &'a [u8],
    /// Scheme.
    pub scheme: &'a str,
    /// Path.
    pub path: &'a str,
    /// Query string, if any.
    pub query_string: Option<&'a str>,
    /// Fragment, if any.
    pub fragment: Option<&'a str>,
    /// Canonicalised URL string.
    pub canonical_url: &'a str,
    /// What canonicalisation changed.
    pub normalised: Value,
}

/// Fields needed to insert or update a link edge.
#[derive(Debug)]
pub struct NewLink<'a> {
    /// Source document (logical reference).
    pub source_doc_id: Uuid,
    /// Target URL.
    pub target_url_id: Uuid,
    /// Target document, once indexed.
    pub target_doc_id: Option<Uuid>,
    /// Anchor text.
    pub anchor_text: Option<&'a str>,
    /// `body` | `nav` | `footer` | `main`.
    pub context: Option<&'a str>,
    /// Whether the link is `nofollow`.
    pub is_nofollow: bool,
    /// Whether the link is sponsored.
    pub is_sponsored: bool,
}

/// Insert a domain or return the existing one.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn upsert_domain(pool: &PgPool, name: &str) -> Result<DomainRow, StorageError> {
    let row = sqlx::query_as::<_, DomainRow>(
        "INSERT INTO domain (name) VALUES ($1)
         ON CONFLICT (name) DO UPDATE SET updated_at = now()
         RETURNING id, name, first_seen, updated_at",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Insert a host or return the existing one.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn upsert_host(
    pool: &PgPool,
    domain_id: Uuid,
    name: &str,
) -> Result<HostRow, StorageError> {
    let row = sqlx::query_as::<_, HostRow>(
        "INSERT INTO host (domain_id, name) VALUES ($1, $2)
         ON CONFLICT (name) DO UPDATE SET updated_at = now()
         RETURNING id, domain_id, name, rate_limit_seconds, requests_in_window,
                   next_request_at, concurrency_limit, consecutive_failures,
                   first_seen, updated_at",
    )
    .bind(domain_id)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Insert a URL or refresh its `last_seen`.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn upsert_url(pool: &PgPool, new: &NewUrl<'_>) -> Result<UrlRow, StorageError> {
    let row = sqlx::query_as::<_, UrlRow>(
        "INSERT INTO url
           (url_hash, scheme, host_id, path, query_string, fragment, canonical_url, normalised)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (url_hash) DO UPDATE SET last_seen = now(), updated_at = now()
         RETURNING id, url_hash, scheme, host_id, path, query_string, fragment,
                   canonical_url, normalised, first_seen, last_seen",
    )
    .bind(new.url_hash)
    .bind(new.scheme)
    .bind(new.host_id)
    .bind(new.path)
    .bind(new.query_string)
    .bind(new.fragment)
    .bind(new.canonical_url)
    .bind(&new.normalised)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Look a URL up by its canonical hash.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn find_url_by_hash(
    pool: &PgPool,
    url_hash: &[u8],
) -> Result<Option<UrlRow>, StorageError> {
    let row = sqlx::query_as::<_, UrlRow>(
        "SELECT id, url_hash, scheme, host_id, path, query_string, fragment,
                canonical_url, normalised, first_seen, last_seen
         FROM url WHERE url_hash = $1",
    )
    .bind(url_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Create the document label row for a URL if it does not exist.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn upsert_document(
    pool: &PgPool,
    doc_id: Uuid,
    url_id: Uuid,
) -> Result<DocumentRow, StorageError> {
    let row = sqlx::query_as::<_, DocumentRow>(
        "INSERT INTO document (doc_id, url_id) VALUES ($1, $2)
         ON CONFLICT (doc_id) DO UPDATE SET url_id = EXCLUDED.url_id, updated_at = now()
         RETURNING doc_id, url_id, state, content_hash, cluster_id, lang,
                   published_at, modified_at, content_type, quality_score,
                   spam_score, indexed_generation, first_indexed_at,
                   last_changed_at, created_at, updated_at",
    )
    .bind(doc_id)
    .bind(url_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Append a document version, ignoring an identical replay.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn insert_document_version(
    pool: &PgPool,
    doc_id: Uuid,
    version: i32,
    content_hash: &[u8],
    status_code: Option<i32>,
    extracted_text: Option<&str>,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO document_version
           (doc_id, version, content_hash, status_code, extracted_text)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (doc_id, version) DO NOTHING",
    )
    .bind(doc_id)
    .bind(version)
    .bind(content_hash)
    .bind(status_code)
    .bind(extracted_text)
    .execute(pool)
    .await?;
    Ok(())
}

/// Insert or refresh a link edge.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn upsert_link(pool: &PgPool, new: &NewLink<'_>) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO link
           (source_doc_id, target_url_id, target_doc_id, anchor_text,
            is_nofollow, is_sponsored, context)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (source_doc_id, target_url_id, is_nofollow)
         DO UPDATE SET last_seen = now(), anchor_text = EXCLUDED.anchor_text",
    )
    .bind(new.source_doc_id)
    .bind(new.target_url_id)
    .bind(new.target_doc_id)
    .bind(new.anchor_text)
    .bind(new.is_nofollow)
    .bind(new.is_sponsored)
    .bind(new.context)
    .execute(pool)
    .await?;
    Ok(())
}

/// Insert or refresh a host's robots policy.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn upsert_robots_policy(
    pool: &PgPool,
    host_id: Uuid,
    raw: Option<&str>,
    parsed: &Value,
    status: Option<i32>,
    expires_at: OffsetDateTime,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO robots_policy (host_id, raw, parsed, status, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (host_id) DO UPDATE SET
           raw = EXCLUDED.raw, parsed = EXCLUDED.parsed, status = EXCLUDED.status,
           fetched_at = now(), expires_at = EXCLUDED.expires_at, parse_error = NULL",
    )
    .bind(host_id)
    .bind(raw)
    .bind(parsed)
    .bind(status)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Enqueue a URL for crawling, returning the frontier entry id.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn enqueue(
    pool: &PgPool,
    url_id: Uuid,
    host_id: Uuid,
    priority: f32,
    depth: i16,
    discovery: DiscoverySource,
) -> Result<Uuid, StorageError> {
    let row = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO crawl_frontier (url_id, host_id, priority, depth, discovery)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id",
    )
    .bind(url_id)
    .bind(host_id)
    .bind(priority)
    .bind(depth)
    .bind(discovery)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Claim a batch of frontier entries for one worker.
///
/// `FOR UPDATE SKIP LOCKED` gives contention-free claiming without a broker, and the
/// join to `host` enforces politeness in the query itself: a worker cannot exceed a
/// host's rate because the claim will not return rows it is not entitled to.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn claim(
    pool: &PgPool,
    worker: &str,
    limit: i64,
) -> Result<Vec<CrawlFrontierRow>, StorageError> {
    let rows = sqlx::query_as::<_, CrawlFrontierRow>(
        "WITH claimable AS (
           SELECT f.id, f.created_at
           FROM crawl_frontier f
           JOIN host h ON h.id = f.host_id
           WHERE f.state IN ('pending', 'ready')
             AND f.not_before <= now()
             AND h.next_request_at <= now()
             AND h.requests_in_window < h.rate_limit_seconds
           ORDER BY f.priority DESC, f.not_before ASC, f.created_at ASC
           LIMIT $1
           FOR UPDATE OF f SKIP LOCKED
         )
         UPDATE crawl_frontier f
         SET state = 'claimed', claimed_by = $2, claimed_at = now()
         FROM claimable c
         WHERE f.id = c.id AND f.created_at = c.created_at
         RETURNING f.id, f.url_id, f.host_id, f.priority, f.depth, f.state,
                   f.not_before, f.attempts, f.generation, f.discovery, f.created_at",
    )
    .bind(limit)
    .bind(worker)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Record one crawl attempt.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn insert_crawl_attempt(
    pool: &PgPool,
    url_id: Uuid,
    host_id: Uuid,
    outcome: &str,
    status_code: Option<i32>,
    bytes: Option<i32>,
    duration_ms: Option<i32>,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO crawl_attempt
           (url_id, host_id, outcome, status_code, bytes, duration_ms)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(url_id)
    .bind(host_id)
    .bind(outcome)
    .bind(status_code)
    .bind(bytes)
    .bind(duration_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a completed crawl attempt row and return it (for tests and diagnostics).
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn latest_attempt(
    pool: &PgPool,
    url_id: Uuid,
) -> Result<Option<CrawlAttemptRow>, StorageError> {
    let row = sqlx::query_as::<_, CrawlAttemptRow>(
        "SELECT id, url_id, host_id, started_at, finished_at, status_code,
                content_type, bytes, duration_ms, outcome, error_class::text AS error_class
         FROM crawl_attempt WHERE url_id = $1
         ORDER BY started_at DESC LIMIT 1",
    )
    .bind(url_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Fields describing a sealed index generation.
#[derive(Debug)]
pub struct NewGeneration<'a> {
    /// Generation number.
    pub generation: i64,
    /// Number of shards.
    pub shard_count: i32,
    /// Document count.
    pub doc_count: i64,
    /// Index schema version.
    pub schema_version: &'a str,
    /// Tokenizer version.
    pub tokenizer_version: &'a str,
    /// Ranking configuration version.
    pub ranking_config_version: &'a str,
    /// Directory of the sealed generation.
    pub path: &'a str,
}

/// Point the index at a freshly sealed generation.
///
/// Flips `is_current` in one transaction so readers never observe zero or two
/// current generations.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the query fails.
pub async fn publish_generation(
    pool: &PgPool,
    new: &NewGeneration<'_>,
) -> Result<(), StorageError> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE index_manifest SET is_current = false WHERE is_current")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO index_manifest
           (generation, shard_count, doc_count, schema_version,
            tokenizer_version, ranking_config_version, path, is_current)
         VALUES ($1, $2, $3, $4, $5, $6, $7, true)
         ON CONFLICT (generation) DO UPDATE SET
           shard_count = EXCLUDED.shard_count, doc_count = EXCLUDED.doc_count,
           schema_version = EXCLUDED.schema_version,
           tokenizer_version = EXCLUDED.tokenizer_version,
           ranking_config_version = EXCLUDED.ranking_config_version,
           path = EXCLUDED.path, is_current = true",
    )
    .bind(new.generation)
    .bind(new.shard_count)
    .bind(new.doc_count)
    .bind(new.schema_version)
    .bind(new.tokenizer_version)
    .bind(new.ranking_config_version)
    .bind(new.path)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
