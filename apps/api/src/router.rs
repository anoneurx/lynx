//! The application router, shared by the binary and the tests.

use std::sync::Arc;

use axum::{routing::get, Router};
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::state::AppState;

/// Body size limit. Search is a GET surface, but the layer is what makes an oversized
/// request cheap to reject rather than expensive to parse.
pub const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Build the router: `/api/v1/search` plus the middleware from Step 6 scope
/// (request tracing, body limit).
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/search", get(crate::handlers::search))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(RequestBodyLimitLayer::new(BODY_LIMIT_BYTES)),
        )
        .with_state(state)
}
