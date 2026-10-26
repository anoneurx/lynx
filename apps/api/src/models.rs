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

impl SearchQuery {
    /// Parse the raw query string with tolerance.
    ///
    /// Every value that fails to parse falls back to its documented default, and bytes
    /// that are not UTF-8 are decoded lossily. A request that survives parsing therefore
    /// always reaches the search path; the only hard rejection is an empty `q`, which
    /// the handler turns into `INVALID_QUERY`.
    #[must_use]
    pub fn from_raw_query(raw: Option<&str>) -> Self {
        let mut this = Self {
            q: String::new(),
            count: default_count(),
            offset: 0,
            cursor: None,
            explain: false,
            format: default_format(),
            stream: false,
        };
        let Some(raw) = raw else { return this };
        for pair in raw.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let key = decode_component(key);
            let value = decode_component(value);
            match key.as_str() {
                "q" => this.q = value,
                "count" => {
                    if let Ok(n) = value.trim().parse() {
                        this.count = n;
                    }
                }
                "offset" => {
                    if let Ok(n) = value.trim().parse() {
                        this.offset = n;
                    }
                }
                "cursor" => this.cursor = Some(value),
                "explain" => this.explain = matches!(value.as_str(), "1" | "true" | "yes"),
                "format" => this.format = value.to_ascii_lowercase(),
                "stream" => this.stream = matches!(value.as_str(), "1" | "true" | "yes"),
                _ => {}
            }
        }
        this
    }
}

/// Percent-decode a single key or value: `+` decodes to a space, `%XX` to its byte, and
/// invalid UTF-8 sequences are replaced (`U+FFFD`), never rejected.
fn decode_component(value: &str) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < raw.len() => {
                let hex = std::str::from_utf8(&raw[i + 1..i + 3]).unwrap_or("zz");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    i += 3;
                } else {
                    bytes.push(b'%');
                    i += 1;
                }
            }
            b => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
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
        explain: bool,
        ranked: &[RankedResult<'_>],
        generation: &IndexGeneration,
        total: u64,
        took_ms: u64,
        next_cursor: Option<String>,
    ) -> Self {
        let mut results = Vec::with_capacity(ranked.len());
        for r in ranked {
            let c: &Candidate = r.candidate;
            let signals = explain.then_some(SignalVector {
                lexical: r.signals.lexical,
                freshness: r.signals.freshness,
                quality: r.signals.quality,
                authority: r.signals.authority,
                context: r.signals.context,
                spam: r.signals.spam_score,
                negation: r.signals.negation,
                negation_penalty: r.negation_penalty,
            });
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
                signals,
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
        // Notices come straight from the parse pipeline (e.g. truncation); the first
        // one is the most actionable, and the full set stays in the plan for explain.
        let notice = plan.notices.first().cloned();
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
                notice,
                truncated: plan.truncated.then_some(true),
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
        Operator::Inanchor => "inanchor",
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

/// Default number of completions per suggestions response.
pub const SUGGEST_DEFAULT_LIMIT: usize = 8;
/// Hard cap on completions per suggestions response.
pub const SUGGEST_MAX_LIMIT: usize = 10;
/// The longest prefix a suggestions request accepts; longer input is truncated.
pub const SUGGEST_MAX_PREFIX_BYTES: usize = 128;

/// The `GET /api/v1/suggestions` request parameters, parsed with the same tolerance as
/// search: unparseable values fall back to defaults, never a 500.
#[derive(Debug)]
pub struct SuggestQuery {
    pub q: String,
    pub limit: usize,
    pub scope: String,
    pub language: Option<String>,
}

impl SuggestQuery {
    /// Parse the raw query string, and seed the interface language from
    /// `Accept-Language` when the explicit `interface_lang` parameter is absent.
    #[must_use]
    pub fn from_raw_query(raw: Option<&str>, accept_language: Option<&str>) -> Self {
        let mut this = Self {
            q: String::new(),
            limit: SUGGEST_DEFAULT_LIMIT,
            scope: "all".to_string(),
            language: None,
        };
        if let Some(raw) = raw {
            for pair in raw.split('&') {
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                let key = decode_component(key);
                let value = decode_component(value);
                match key.as_str() {
                    "q" => this.q = value,
                    "limit" => {
                        if let Ok(n) = value.trim().parse::<usize>() {
                            this.limit = n;
                        }
                    }
                    "scope" => this.scope = value.to_ascii_lowercase(),
                    "interface_lang" | "lang" => this.language = Some(value),
                    _ => {}
                }
            }
        }
        if this.language.is_none() {
            this.language = first_accept_language(accept_language);
        }
        this
    }

    /// The effective limit, clamped to the documented range.
    #[must_use]
    pub fn clamped_limit(&self) -> usize {
        self.limit.clamp(1, SUGGEST_MAX_LIMIT)
    }

    /// The prefix to complete: trimmed, then truncated at 128 bytes on a character
    /// boundary (`docs/api/suggestions.md` request contract).
    #[must_use]
    pub fn prefix(&self) -> String {
        let trimmed = self.q.trim();
        if trimmed.len() <= SUGGEST_MAX_PREFIX_BYTES {
            return trimmed.to_string();
        }
        let mut end = SUGGEST_MAX_PREFIX_BYTES;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed[..end].to_string()
    }

    /// Why the response may carry a `meta.notice`, per the scope contract.
    #[must_use]
    pub fn scope_notice(&self) -> Option<&'static str> {
        match self.scope.as_str() {
            "all" | "titles" | "terms" => None,
            "related" => Some("related suggestions are a page-level feature in v0.1 and are not served by this endpoint"),
            _ => Some("unknown scope; treated as all"),
        }
    }

    /// Whether the scope admits term candidates. Unknown scopes are treated as `all`
    /// per the contract; `related` never reaches filtering (it short-circuits to empty).
    #[must_use]
    pub fn wants_terms(&self) -> bool {
        self.scope.as_str() != "titles"
    }

    /// Whether the scope admits title candidates.
    #[must_use]
    pub fn wants_titles(&self) -> bool {
        self.scope.as_str() != "terms"
    }
}

/// The first language tag of an `Accept-Language` header, lowercased; `None` when the
/// header is absent or the first entry is not a plain language tag.
fn first_accept_language(value: Option<&str>) -> Option<String> {
    let tag = value?
        .split(',')
        .next()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    (tag.chars().all(|c| c.is_ascii_graphic()) && tag.len() <= 32).then_some(tag)
}

/// Query echo block of the suggestions response.
#[derive(Debug, Serialize)]
pub struct SuggestQueryInfo {
    pub prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// One ranked completion.
#[derive(Debug, Serialize)]
pub struct SuggestionItem {
    pub text: String,
    /// `terms` or `titles`, matching `docs/api/suggestions.md`.
    pub kind: String,
    /// Corpus frequency prior, 0…1. **Not** query frequency, which does not exist.
    pub prior: f64,
    /// Corpus support: how many documents back this string.
    pub doc_count: u32,
}

/// A related-search entry. The list is always empty in v0.1 (related is page-level),
/// but the field exists so clients do not need a schema change to adopt it.
#[derive(Debug, Serialize)]
pub struct RelatedItem {
    pub text: String,
    pub doc_count: u32,
}

/// Response envelope for `GET /api/v1/suggestions`.
#[derive(Debug, Serialize)]
pub struct SuggestResponse {
    pub query: SuggestQueryInfo,
    pub suggestions: Vec<SuggestionItem>,
    pub related: Vec<RelatedItem>,
    pub meta: SuggestMeta,
}

#[derive(Debug, Serialize)]
pub struct SuggestMeta {
    pub took_ms: u64,
    pub index_generation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<&'static str>,
}

impl SuggestResponse {
    /// Build the response. Completions have already been ranked by `lynx-suggestions`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // priors are 0…1 floats by construction
    pub fn from_completions(
        prefix: &str,
        language: Option<String>,
        completions: Vec<lynx_suggestions::Completion>,
        generation: &IndexGeneration,
        took_ms: u64,
        notice: Option<&'static str>,
    ) -> Self {
        let suggestions = completions
            .into_iter()
            .map(|c| SuggestionItem {
                text: c.text,
                kind: c.kind.as_str().to_string(),
                // Three decimals are ample for a 0…1 prior and keep JSON deterministic.
                prior: ((f64::from(c.prior) * 1_000.0).round() / 1_000.0),
                doc_count: c.doc_count,
            })
            .collect();
        Self {
            query: SuggestQueryInfo {
                prefix: prefix.to_string(),
                language,
            },
            suggestions,
            related: Vec::new(),
            meta: SuggestMeta {
                took_ms,
                index_generation: format!("gen-{}", generation.id),
                notice,
            },
        }
    }

    /// The empty-but-successful response for short prefixes: a normal answer, not an
    /// error (`docs/api/suggestions.md` "Empty and short prefixes").
    #[must_use]
    pub fn empty(
        prefix: &str,
        language: Option<String>,
        generation: &IndexGeneration,
        took_ms: u64,
    ) -> Self {
        Self {
            query: SuggestQueryInfo {
                prefix: prefix.to_string(),
                language,
            },
            suggestions: Vec::new(),
            related: Vec::new(),
            meta: SuggestMeta {
                took_ms,
                index_generation: format!("gen-{}", generation.id),
                notice: None,
            },
        }
    }
}
