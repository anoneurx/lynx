//! Handlers for the minimal API surface.

use std::sync::Arc;
use std::time::Instant;

use axum::{extract::Query, extract::State, Json};
use lynx_common::time::Clock;
// SearchIndexReader is trait in state, not used directly here.

use crate::errors::ApiErrorKind;
use crate::models::SearchQuery;
use crate::state::AppState;

fn default_clock() -> Arc<dyn Clock> {
    Arc::new(lynx_common::time::SystemClock)
}

#[allow(clippy::unused_async)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<crate::models::SearchResponse>, ApiErrorKind> {
    let start = Instant::now();
    let clock = state.clock.clone().unwrap_or_else(default_clock);
    let now = clock.now();

    if params.q.is_empty() {
        return Err(ApiErrorKind::InvalidQuery);
    }

    let plan = lynx_query::parse(&params.q);
    if plan.truncated {
        // We still run, but mark via notice; minimal API returns 200 in practice? But spec says
        // "Query truncated" can appear as notice. For minimal, still continue to search.
    }

    let opts = params.to_search_options(now);
    let candidates = state
        .reader
        .search(&plan, &opts)
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let stats = state
        .reader
        .collection_stats()
        .map_err(|_| ApiErrorKind::IndexUnavailable)?;
    let gen = state.reader.generation();

    let ranked = state.ranker.rank(&plan, &candidates, &stats, now);

    let resp = crate::models::SearchResponse::from_ranked(
        &plan,
        &ranked,
        &gen,
        &stats,
        start
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        plan.truncated,
    );

    Ok(Json(resp))
}
