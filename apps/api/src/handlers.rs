//! Handlers for the minimal API surface.

use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use lynx_common::time::Clock;

use crate::errors::ApiErrorKind;
use crate::models::SearchQuery;
use crate::state::AppState;

fn default_clock() -> Arc<dyn Clock> {
    Arc::new(lynx_common::time::SystemClock)
}

/// `GET /api/v1/search`.
///
/// Content negotiation: `format=json` (default) or `format=html`. The HTML representation
/// is server-rendered and fully usable without JavaScript (`docs/api/search.md`).
///
/// # Errors
/// Returns [`ApiErrorKind::InvalidQuery`] when `q` is empty, and
/// [`ApiErrorKind::IndexUnavailable`] when the index cannot be read.
#[allow(clippy::unused_async)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> Result<impl IntoResponse, ApiErrorKind> {
    let start = Instant::now();
    let clock = state.clock.clone().unwrap_or_else(default_clock);
    let now = clock.now();

    if params.q.is_empty() {
        return Err(ApiErrorKind::InvalidQuery);
    }

    let plan = lynx_query::parse(&params.q);

    let opts = params.to_search_options(now);
    let candidates = state
        .reader
        .search(&plan, &opts)
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let collection = state
        .reader
        .collection_stats()
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let gen = state.reader.generation();

    let ranked = state.ranker.rank(&plan, &candidates, &collection, now);

    let resp = crate::models::SearchResponse::from_ranked(
        &plan,
        &ranked,
        &gen,
        &collection,
        start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        plan.truncated,
    );

    let format = params.format.to_ascii_lowercase();
    if format == "html" {
        let html = crate::serp::render_html(&resp, &params.q, params.offset, params.count);
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
