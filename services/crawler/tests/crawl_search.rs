//! End-to-end pipeline test: crawl → parse → store → index → search.
//!
//! The fixture origin serves a small linked site over loopback. The downloader fetches
//! each page, the parser turns bytes into a bounded `ParsedDocument`, the index pipeline
//! records a version and its edges through a document store, and the sealed generation is
//! searched through the Tantivy-backed reader. This is the integration seam Step 12 is
//! meant to prove: every stage is the real one, only the origin and the store are
//! in-process.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use lynx_crawler::downloader::Downloader;
use lynx_crawler::safety::{FetchPolicy, ResourceLimits, StaticResolver};
use lynx_fixture_origin::bind_ephemeral;
use lynx_indexer::{
    AddOutcome, IndexPipeline, MemoryDocumentStore, SearchIndexReader, SearchOptions,
    TantivyIndexReader,
};
use lynx_parser::parse;
use lynx_query::parse as parse_query;
use time::macros::datetime;

fn downloader(port: u16) -> Downloader {
    let limits = ResourceLimits {
        max_response_bytes: 64 * 1024,
        max_decompression_ratio: 100,
        connect_timeout_ms: 2_000,
        header_timeout_ms: 2_000,
        read_idle_timeout_ms: 2_000,
        total_timeout_ms: 10_000,
    };
    let policy = FetchPolicy::new(vec![port], 5, limits)
        .unwrap()
        .dangerously_allow_private_addresses();
    Downloader::new(policy, Arc::new(StaticResolver::new()))
}

async fn server() -> (u16, String) {
    let (addr, handle) = bind_ephemeral().await.unwrap();
    drop(handle);
    (addr.port(), format!("http://{addr}"))
}

#[tokio::test]
async fn crawls_parses_indexes_and_searches_a_site() {
    let (port, base) = server().await;
    let fetch = downloader(port);

    let slugs = ["rust-async", "tokio-runtime", "noindex-draft"];
    let store = MemoryDocumentStore::new();
    let mut pipeline = IndexPipeline::new(store, datetime!(2026-03-01 00:00 UTC));

    let mut indexed = 0;
    for slug in slugs {
        let response = fetch
            .fetch(&format!("{base}/articles/{slug}"))
            .await
            .unwrap();
        assert_eq!(response.status, 200);

        let html = String::from_utf8_lossy(&response.body);
        let parsed = parse(&html, &response.final_url);

        let outcome = pipeline
            .ingest(parsed, Some(response.status))
            .await
            .unwrap();
        if outcome.outcome == AddOutcome::Indexed {
            indexed += 1;
        }
    }

    assert_eq!(indexed, 2, "the noindex draft must not be indexed");
    assert_eq!(
        pipeline.store().versions().len(),
        3,
        "all fetches are versioned"
    );
    assert_eq!(
        pipeline.store().links().len(),
        2,
        "rust-async carries two outbound links"
    );

    let (docs, generation) = pipeline.seal(1);
    let reader = TantivyIndexReader::from_documents(docs, generation).unwrap();

    let opts = SearchOptions {
        now: datetime!(2026-03-01 00:00 UTC),
        ..SearchOptions::default()
    };

    // A phrase unique to the linked page ranks that page.
    let hits = reader.search(&parse_query("event loop"), &opts).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Tokio Runtime");

    // The linking page is found by its prose, and its metadata survived the round trip.
    let hits = reader
        .search(&parse_query("concurrency threads"), &opts)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Rust Async Guide");
    assert_eq!(
        hits[0].published_at,
        Some(datetime!(2026-02-18 09:00:00 UTC))
    );
    assert!(hits[0].link_count >= 2);

    // The noindex page was never added to the index.
    let hits = reader.search(&parse_query("quokka"), &opts).unwrap();
    assert!(hits.is_empty(), "noindex content must not be searchable");
}
