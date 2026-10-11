//! `lynx-api` — the public HTTP API.

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
use lynx_indexer::InMemoryIndex;
use lynx_ranker::Ranker;
use time::macros::datetime;
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::info;

use lynx_api::state::AppState;

#[tokio::main]
async fn main() {
    let opts = lynx_logging::Options::default();
    if let Err(error) = lynx_logging::init(&opts) {
        eprintln!("failed to initialise logging: {error}");
        return;
    }

    // Minimal bootstrap: create a seeded in-memory index so /search works out of the box.
    // This is a temporary fixture for Step 6; in production, Tantivy will be loaded.
    let mut index = InMemoryIndex::new();
    let mut rust_doc = lynx_indexer::IndexedDocument::new(
        lynx_common::ids::DocumentId::from_canonical_url("https://tokio.rs/"),
        "https://tokio.rs/",
        "Tokio - An asynchronous runtime for Rust",
        "A multi-threaded asynchronous runtime for Rust, with a scheduler, timers, and an I/O driver.",
    );
    rust_doc.lang = Some("en".to_string());
    rust_doc.published_at = Some(datetime!(2025-11-04 00:00 UTC));
    rust_doc.quality_score = 0.9;
    index.add(rust_doc);

    let app_state = Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
    });

    let app = Router::new()
        .route("/api/v1/search", get(lynx_api::handlers::search))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(RequestBodyLimitLayer::new(64 * 1024)),
        )
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    info!(service = "lynx-api", addr = %addr, "starting");
    if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
        let _ = axum::serve(listener, app).await;
    } else {
        tracing::error!("failed to bind to {}", addr);
    }
}
