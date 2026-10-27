//! The application router, shared by the binary and the tests.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::errors::ApiErrorKind;
use crate::state::AppState;

/// Body size limit. Search is a GET surface, but the layer is what makes an oversized
/// request cheap to reject rather than expensive to parse.
pub const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// A trace span carrying only the method and the path. The query string is deliberately
/// excluded: the spec's first privacy rule is that query text never enters a log, trace,
/// or metric, *enforced by construction* — this span simply cannot hold it.
fn redacted_span(req: &Request<Body>) -> tracing::Span {
    let method = req.method().as_str();
    let path = req.uri().path();
    tracing::info_span!("http_request", method, path)
}

/// Build the router with the full middleware stack (spec step 6): redacted tracing,
/// security headers, request ids, rate limiting, and body limits.
pub fn router(state: Arc<AppState>) -> Router {
    let app = Router::new()
        .route("/api/v1/search", get(crate::handlers::search))
        .route("/api/v1/suggestions", get(crate::handlers::suggestions))
        .route("/api/v1/status", get(crate::handlers::status))
        .fallback(not_found)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http().make_span_with(redacted_span))
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::middleware::apply_security_headers,
                ))
                .layer(axum::middleware::from_fn(
                    crate::middleware::attach_request_id,
                ))
                .layer(axum::middleware::from_fn(crate::middleware::privacy_gpc))
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::middleware::rate_limit,
                ))
                .layer(RequestBodyLimitLayer::new(BODY_LIMIT_BYTES)),
        );
    app.with_state(state)
}

/// Fallback for unmatched paths: the standard error envelope, not plain text.
///
/// # Errors
/// Always returns the `NOT_FOUND` envelope; this function is the 404 path.
async fn not_found() -> (axum::http::StatusCode, axum::response::Response) {
    let resp = ApiErrorKind::NotFound.into_response();
    (axum::http::StatusCode::NOT_FOUND, resp)
}
