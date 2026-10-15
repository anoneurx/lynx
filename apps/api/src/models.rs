//! Request/response DTOs for the minimal search API.

use lynx_indexer::{Candidate, FieldWeights, IndexGeneration, SearchOptions};
use lynx_query::{Operator, QueryPlan};
use lynx_ranker::RankedResult;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// How many candidates retrieval fetches before ranking. Ranking then reorders this window
/// and the handler pages through it, so pagination is consistent within a generation.
pub const CANDIDATE_WINDOW: usize = 2000;

/// Deep offset pagination is capped; beyond this, clients use the cursor or refine.
pub const MAX_OFFSET: usize = 1000;

/// Maximum page size a single request may ask for.
pub const MAX_PAGE: usize = 100;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_count")]
    pub count: usize,
    #[serde(default)]
    pub offset: usize,
    /// Opaque keyset cursor from a previous response's `meta.next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub explain: bool,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default)]
    pub stream: bool,
}

fn default_count() -> usize {
    10
}

fn default_format() -> String {
    "json".to_string()
}

impl SearchQuery {
    /// The number of results to return on this page, clamped to the documented maximum.
    #[must_use]
    pub fn page_size(&self) -> usize {
        self.count.clamp(1, MAX_PAGE)
    }

    /// Retrieval options for the full candidate window. Offset and cursor are applied after
    /// ranking, so retrieval always fetches from the front of the ranked sequence.
    #[must_use]
    pub fn candidate_options(&self, now: OffsetDateTime) -> SearchOptions {
        SearchOptions {
            limit: CANDIDATE_WINDOW,
            offset: 0,
            now,
            weights: FieldWeights::default(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct QueryInfo {
    pub terms: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub corrections: Vec<serde_json::Value>,
    pub expansions: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operators_used: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct SignalVector {
    pub lexical: f64,
    pub freshness: f64,
    pub quality: f64,
    pub authority: f64,
    pub context: f64,
    pub spam: f64,
    /// Negation intensity in `[0, 1]`; `0` when the query had no negative terms.
    pub negation: f64,
    /// The multiplicative negation penalty applied.
    pub negation_penalty: f64,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub rank: usize,
    pub doc_id: String,
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signals: Option<SignalVector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SearchMeta {
    /// Number of documents matching the query, not the corpus size.
    pub total: u64,
    pub returned: usize,
    pub took_ms: u64,
    pub index_generation: String,
    pub ranking_config_version: String,
    pub degraded: bool,
    /// Opaque cursor for the next page. Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: QueryInfo,
    pub results: Vec<SearchResult>,
    pub meta: SearchMeta,
}

impl SearchResponse {
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::too_many_arguments)] // a response is assembled from distinct, independent parts
    #[must_use]
    pub fn from_ranked(
        plan: &QueryPlan,
        ranked: &[RankedResult<'_>],
        generation: &IndexGeneration,
        total: u64,
        took_ms: u64,
        truncated: bool,
        next_cursor: Option<String>,
    ) -> Self {
        let mut results = Vec::with_capacity(ranked.len());
        for r in ranked {
            let c: &Candidate = r.candidate;
            let signals = SignalVector {
                lexical: r.signals.lexical,
                freshness: r.signals.freshness,
                quality: r.signals.quality,
                authority: r.signals.authority,
                context: r.signals.context,
                spam: r.signals.spam_score,
                negation: r.signals.negation,
                negation_penalty: r.negation_penalty,
            };
            results.push(SearchResult {
                rank: r.rank,
                doc_id: c.doc_id.to_string(),
                url: c.url.clone(),
                title: c.title.clone(),
                snippet: c
                    .body
                    .chars()
                    .take(180)
                    .collect::<String>()
                    .replace('\n', " "),
                score: r.score,
                published_at: c.published_at.map(|d| d.date().to_string()),
                content_type: c.content_type.clone(),
                signals: Some(signals),
                cluster_id: r.signals.cluster_id,
            });
        }
        let query_info = QueryInfo {
            terms: plan.term_count(),
            intent: None,
            language: None,
            confidence: None,
            corrections: Vec::new(),
            expansions: Vec::new(),
            operators_used: Some(
                plan.operators_used
                    .iter()
                    .map(|op| operator_name(*op).to_string())
                    .collect(),
            ),
        };
        let returned = results.len();
        SearchResponse {
            query: query_info,
            results,
            meta: SearchMeta {
                total,
                returned,
                took_ms,
                index_generation: format!("gen-{}", generation.id),
                ranking_config_version: generation.ranking_config_version.clone(),
                degraded: false,
                next_cursor,
                notice: None,
                truncated: if truncated { Some(true) } else { None },
            },
        }
    }
}

/// The stable, lowercase name of an operator, matching its serde representation.
fn operator_name(op: Operator) -> &'static str {
    match op {
        Operator::Phrase => "phrase",
        Operator::Exclude => "exclude",
        Operator::Site => "site",
        Operator::Inurl => "inurl",
        Operator::Intitle => "intitle",
        Operator::Ext => "ext",
        Operator::Before => "before",
        Operator::After => "after",
        Operator::Freshness => "freshness",
        Operator::Filetype => "filetype",
        Operator::Lang => "lang",
        Operator::Or => "or",
        Operator::Force => "force",
    }
}

/// Encode a ranked result's keyset position `(score, doc_id)` as an opaque cursor.
#[must_use]
pub fn encode_cursor(result: &RankedResult<'_>) -> String {
    format!(
        "{:016x}:{}",
        result.score.to_bits(),
        result.candidate.doc_id
    )
}

/// Decode a cursor back to `(score, doc_id)`. `None` for a malformed cursor, which callers
/// treat as "start from the beginning" rather than an error.
#[must_use]
pub fn decode_cursor(cursor: &str) -> Option<(f64, String)> {
    let (bits, doc_id) = cursor.split_once(':')?;
    let bits = u64::from_str_radix(bits, 16).ok()?;
    Some((f64::from_bits(bits), doc_id.to_string()))
}

/// `GET /api/v1/status` response (`docs/api/status.md`).
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// `healthy`, `degraded`, `maintenance`, or `unavailable`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexStatus>,
    pub quality: QualityStatus,
    pub service: ServiceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// Index freshness and corpus size.
#[derive(Debug, Serialize)]
pub struct IndexStatus {
    pub generation: String,
    pub generation_age_hours: f64,
    pub documents: u64,
    pub bytes: u64,
    pub shards: u32,
}

/// Aggregate ranking-quality figures. Averages that require a scan are omitted in v0.1.
#[derive(Debug, Serialize)]
pub struct QualityStatus {
    pub ranking_config_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_quality_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spam_flagged_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_rate_top10: Option<f64>,
}

/// Service-level health.
#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub version: String,
    pub uptime_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_p50_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_p95_ms: Option<u64>,
    pub degraded: bool,
}

impl StatusResponse {
    /// Build a status response from the served generation and process timing.
    ///
    /// `degraded` reports an index that answered with an error elsewhere in the request
    /// path; it does not stop the endpoint from returning `200`, per `docs/api/status.md`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // hours are well within f64's exact range
    #[allow(clippy::cast_sign_loss)] // clamped non-negative before the cast
    pub fn build(
        generation: &IndexGeneration,
        degraded: bool,
        now: OffsetDateTime,
        started_at: OffsetDateTime,
        version: &str,
    ) -> Self {
        let age_seconds = (now - generation.created_at).as_seconds_f64().max(0.0);
        let uptime = (now - started_at).whole_seconds().max(0);
        Self {
            status: if degraded { "degraded" } else { "healthy" },
            index: Some(IndexStatus {
                generation: format!("gen-{}", generation.id),
                generation_age_hours: age_seconds / 3600.0,
                documents: generation.doc_count,
                bytes: generation.size_bytes,
                shards: generation.shards,
            }),
            quality: QualityStatus {
                ranking_config_version: generation.ranking_config_version.clone(),
                avg_quality_score: None,
                spam_flagged_rate: None,
                duplicate_rate_top10: None,
            },
            service: ServiceStatus {
                version: version.to_string(),
                uptime_seconds: uptime as u64,
                latency_p50_ms: None,
                latency_p95_ms: None,
                degraded,
            },
            notice: None,
        }
    }
}
