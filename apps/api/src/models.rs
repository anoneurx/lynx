//! Request/response DTOs for the minimal search API.

use lynx_indexer::{Candidate, CollectionStats, IndexGeneration, SearchOptions};
use lynx_query::QueryPlan;
use lynx_ranker::RankedResult;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_count")]
    pub count: usize,
    #[serde(default)]
    pub offset: usize,
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
    #[must_use]
    pub fn to_search_options(&self, now: OffsetDateTime) -> SearchOptions {
        SearchOptions {
            limit: self.count.min(100),
            offset: self.offset,
            now,
            weights: lynx_indexer::FieldWeights::default(),
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
    pub total: u64,
    pub returned: usize,
    pub took_ms: u64,
    pub index_generation: String,
    pub ranking_config_version: String,
    pub degraded: bool,
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
    #[must_use]
    pub fn from_ranked(
        plan: &QueryPlan,
        ranked: &[RankedResult<'_>],
        generation: &IndexGeneration,
        stats: &CollectionStats,
        took_ms: u64,
        truncated: bool,
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
            operators_used: None,
        };
        let returned = results.len();
        SearchResponse {
            query: query_info,
            results,
            meta: SearchMeta {
                total: stats.doc_count,
                returned,
                took_ms,
                index_generation: format!("gen-{}", generation.id),
                ranking_config_version: generation.ranking_config_version.clone(),
                degraded: false,
                notice: None,
                truncated: if truncated { Some(true) } else { None },
            },
        }
    }
}
