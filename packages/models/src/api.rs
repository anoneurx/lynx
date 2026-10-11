//! Public API contract types.
//!
//! These are the only domain-shaped types that may be serialised to a client. Each
//! carries an explicit, reviewed set of fields; adding one is a deliberate act.

use serde::{Deserialize, Serialize};

use crate::domain::DocumentId;

/// A single search result, as served.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Opaque document id (safe to expose; not reversible to a query).
    pub id: DocumentId,
    /// Result URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Snippet, with `<mark>` around matched terms.
    pub snippet: String,
    /// Detected language, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// The full signal breakdown, retained so ranking is explainable.
    pub signals: SignalBreakdown,
}

/// Per-signal contributions to a result's score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SignalBreakdown {
    /// Lexical relevance (BM25F).
    pub bm25: f32,
    /// Recency.
    pub freshness: f32,
    /// Static quality.
    pub quality: f32,
    /// Link-graph authority.
    pub authority: f32,
    /// Query/document context match.
    pub context: f32,
    /// Multiplicative spam penalty in `[0,1]`.
    pub spam: f32,
    /// Final combined score.
    pub total: f32,
}

/// A notice: the request succeeded, with a caveat worth stating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    /// Stable notice code.
    pub code: String,
    /// Human message.
    pub message: String,
}

/// Response to `GET /api/v1/search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    /// The ranked results page.
    pub results: Vec<SearchResult>,
    /// Total estimated matches (may be a lower bound).
    pub total: u64,
    /// Opaque cursor for the next page, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Server-side processing time.
    pub took_ms: u64,
    /// Whether the query was truncated before parsing.
    pub truncated: bool,
    /// Whether any shard was degraded.
    pub degraded: bool,
    /// Non-fatal caveats.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<Notice>,
    /// Spelling corrections applied, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<String>,
}

/// Response to `GET /api/v1/suggestions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestionResponse {
    /// Completion candidates, derived from the corpus (never from query logs).
    pub suggestions: Vec<String>,
}

/// Response to `GET /api/v1/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusResponse {
    /// Current index generation.
    pub generation: u64,
    /// Number of indexed documents.
    pub documents: u64,
    /// Age of the index in seconds.
    pub index_age_seconds: u64,
    /// Whether the service is currently serving.
    pub serving: bool,
}

/// The single error envelope used by every endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    /// The error body.
    pub error: ErrorBody,
}

/// The error body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Stable, machine-readable code.
    pub code: String,
    /// Human-readable message. Query-free.
    pub message: String,
    /// HTTP status, repeated for clients that only read the body.
    pub status: u16,
    /// Whether retrying can help.
    pub retryable: bool,
    /// Present when `retryable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Endpoint-specific context. Query-free.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub detail: serde_json::Map<String, serde_json::Value>,
}

impl From<lynx_common::LynxError> for ErrorEnvelope {
    fn from(err: lynx_common::LynxError) -> Self {
        let detail = err
            .detail
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect();
        Self {
            error: ErrorBody {
                code: err.code.as_str().to_string(),
                message: err.message,
                status: err.code.status(),
                retryable: err.code.retryable(),
                retry_after_ms: err.retry_after_ms,
                detail,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lynx_common::{ErrorCode, LynxError};

    #[test]
    fn error_envelope_matches_documented_shape() {
        let err = LynxError::new(ErrorCode::RateLimited, "Too many requests.")
            .with_retry_after_ms(42_000)
            .with_detail("limit", "60");
        let env = ErrorEnvelope::from(err);
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["error"]["code"], "RATE_LIMITED");
        assert_eq!(json["error"]["status"], 429);
        assert_eq!(json["error"]["retryable"], true);
        assert_eq!(json["error"]["retry_after_ms"], 42_000);
        assert_eq!(json["error"]["detail"]["limit"], "60");
    }

    #[test]
    fn outbound_search_result_omits_absent_lang() {
        let result = SearchResult {
            id: DocumentId::new(),
            url: "https://e.com/".into(),
            title: "T".into(),
            snippet: "s".into(),
            lang: None,
            signals: SignalBreakdown {
                bm25: 1.0,
                freshness: 0.0,
                quality: 0.0,
                authority: 0.0,
                context: 0.0,
                spam: 0.0,
                total: 1.0,
            },
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("lang").is_none());
    }
}
