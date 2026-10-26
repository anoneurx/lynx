//! Handlers for the minimal API surface.

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{RawQuery, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use lynx_common::time::Clock;
use lynx_ranker::RankedResult;

use crate::errors::ApiErrorKind;
use crate::models::{
    decode_cursor, encode_cursor, SearchQuery, SearchResponse, StatusResponse, MAX_OFFSET,
};
use crate::state::AppState;

fn default_clock() -> Arc<dyn Clock> {
    Arc::new(lynx_common::time::SystemClock)
}

/// `GET /api/v1/search`.
///
/// The raw query string is parsed in-handler with tolerance: values that fail to parse
/// fall back to their documented defaults instead of producing a 500 from a parser
/// rejection, and non-UTF-8 input is decoded lossily (`docs/api/errors.md`: "400 for any
/// client input problem, 5xx only for server faults").
///
/// Content negotiation: `format=json` (default) or `format=html`. The HTML representation
/// is server-rendered and fully usable without JavaScript (`docs/api/search.md`).
///
/// Retrieval fetches a fixed candidate window, ranking reorders it, and pagination (offset
/// or keyset cursor) selects a page from the ranked sequence. `meta.total` is the number of
/// documents matching the query, not the corpus size.
///
/// # Errors
/// Returns [`ApiErrorKind::InvalidQuery`] when `q` is empty, and
/// [`ApiErrorKind::IndexUnavailable`] when the index cannot be read.
#[allow(clippy::unused_async)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
) -> Result<impl IntoResponse, ApiErrorKind> {
    let start = Instant::now();
    let clock = state.clock.clone().unwrap_or_else(default_clock);
    let now = clock.now();

    let params = SearchQuery::from_raw_query(raw.as_deref());
    if params.q.is_empty() {
        return Err(ApiErrorKind::InvalidQuery);
    }

    let plan = lynx_query::parse(&params.q);

    let opts = params.candidate_options(now);
    let candidates = state
        .reader
        .search(&plan, &opts)
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let collection = state
        .reader
        .collection_stats()
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let total = state
        .reader
        .count(&plan)
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let gen = state.reader.generation();

    let ranked = state.ranker.rank(&plan, &candidates, &collection, now);

    // Cursor takes precedence over offset. A malformed cursor is ignored (never an error):
    // the user-facing bias is to show results rather than reject.
    let clamped_offset = params.offset.min(MAX_OFFSET);
    let start_index = match params.cursor.as_deref().and_then(decode_cursor) {
        Some((score, doc_id)) => position_after(&ranked, score, &doc_id),
        None => clamped_offset.min(ranked.len()),
    };
    let page_size = params.page_size();
    let end_index = start_index.saturating_add(page_size).min(ranked.len());
    let page = &ranked[start_index.min(ranked.len())..end_index];
    let next_cursor = if end_index < ranked.len() {
        page.last().map(encode_cursor)
    } else {
        None
    };
    let has_more = next_cursor.is_some();

    let resp = SearchResponse::from_ranked(
        &plan,
        params.explain,
        page,
        &gen,
        total,
        start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        next_cursor,
    );

    let format = params.format.to_ascii_lowercase();
    if format == "html" {
        let html = crate::serp::render_html(&resp, &params.q, clamped_offset, page_size, has_more);
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        ))
    } else {
        let json = serde_json::to_string(&resp).unwrap_or_default();
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        ))
    }
}

/// Index of the first result strictly after the cursor position in ranked order.
///
/// Ranking sorts by score descending, then `doc_id` ascending, so "after" is a lower score,
/// or an equal score with a larger `doc_id`.
fn position_after(ranked: &[RankedResult<'_>], score: f64, doc_id: &str) -> usize {
    ranked
        .iter()
        .position(|r| match r.score.total_cmp(&score) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => r.candidate.doc_id.to_string().as_str() > doc_id,
        })
        .unwrap_or(ranked.len())
}

/// `GET /api/v1/status` (`docs/api/status.md`).
///
/// Public. Reports index freshness, corpus size, and service health. Always `200`, even when
/// degraded, so a client gets a readable answer rather than a retryable error.
#[allow(clippy::unused_async)]
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let clock = state.clock.clone().unwrap_or_else(default_clock);
    let now = clock.now();
    let generation = state.reader.generation();
    // If the index cannot even answer a statistics query, report degradation but still
    // serve the manifest we hold.
    let degraded = state.reader.collection_stats().is_err();
    let resp = StatusResponse::build(
        &generation,
        degraded,
        now,
        state.started_at,
        env!("CARGO_PKG_VERSION"),
    );
    let json = serde_json::to_string(&resp).unwrap_or_default();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=60"),
        ],
        json,
    )
}
