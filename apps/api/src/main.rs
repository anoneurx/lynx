//! `lynx-api` — the public HTTP API.

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use lynx_indexer::InMemoryIndex;
use lynx_ranker::Ranker;
use time::macros::datetime;
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
    // This is a temporary fixture; in production, a Tantivy generation is loaded.
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
        started_at: time::OffsetDateTime::now_utc(),
        security: lynx_api::middleware::SecurityContext::from_env(),
    });

    let app = lynx_api::router::router(app_state);

    let addr = std::env::var("LYNX_API_BIND")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 8080)));
    info!(service = "lynx-api", addr = %addr, "starting");
    if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    } else {
        tracing::error!("failed to bind to {}", addr);
    }
}
