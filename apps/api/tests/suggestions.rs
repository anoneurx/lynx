//! End-to-end tests for the suggestions endpoint, through the real router.
//!
//! These exercise the HTTP surface (`GET /api/v1/suggestions`) as a client sees it:
//! status codes for "ordinary gaps" versus real faults, the scope contract, the cache
//! contract, and the privacy-relevant property that every user's response is identical.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lynx_api::state::AppState;
use lynx_common::ids::DocumentId;
use lynx_indexer::{InMemoryIndex, IndexedDocument};
use lynx_ranker::Ranker;
use time::macros::datetime;
use tower::ServiceExt;

fn seeded() -> Arc<AppState> {
    let mut index = InMemoryIndex::new();

    let mut rust = IndexedDocument::new(
        DocumentId::from_canonical_url("https://tokio.rs/"),
        "https://tokio.rs/",
        "Tokio - An asynchronous runtime for Rust",
        "A multi-threaded asynchronous runtime for Rust, with a scheduler and timers.",
    );
    rust.lang = Some("en".to_string());
    rust.published_at = Some(datetime!(2025-11-04 00:00 UTC));
    rust.quality_score = 0.9;
    index.add(rust);

    let mut zig = IndexedDocument::new(
        DocumentId::from_canonical_url("https://ziglang.org/"),
        "https://ziglang.org/",
        "Zig programming language",
        "Zig is a general-purpose systems programming language.",
    );
    zig.lang = Some("en".to_string());
    zig.published_at = Some(datetime!(2024-01-01 00:00 UTC));
    index.add(zig);

    Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
        started_at: datetime!(2026-02-18 00:00 UTC),
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: None,
        answer_max_sources: 10,
    })
}

async fn get_with_header(
    state: Arc<AppState>,
    uri: &str,
    accept_language: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let app = lynx_api::router::router(state);
    let mut builder = Request::builder().uri(uri);
    if let Some(lang) = accept_language {
        builder = builder.header(header::ACCEPT_LANGUAGE, lang);
    }
    let resp = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn get(state: Arc<AppState>, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    get_with_header(state, uri, None).await
}

#[tokio::test]
async fn completions_are_ranked_items_with_corpus_support() {
    let (status, headers, body) = get(seeded(), "/api/v1/suggestions?q=ru").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=300"
    );

    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["query"]["prefix"], "ru");
    let items = v["suggestions"].as_array().unwrap();
    assert!(!items.is_empty(), "a corpus with `rust` must complete `ru`");
    for item in items {
        assert!(
            ["terms", "titles"].contains(&item["kind"].as_str().unwrap()),
            "kind is the API vocabulary: {item}"
        );
        let prior = item["prior"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&prior), "prior {prior} must be in 0…1");
        assert!(item["doc_count"].as_u64().unwrap() >= 1);
    }
    assert!(items.iter().any(|i| i["text"] == "rust"));
    assert!(v["related"].as_array().unwrap().is_empty());
    assert!(v["meta"]["took_ms"].is_u64());
    assert!(v["meta"]["index_generation"].is_string());
}

#[tokio::test]
async fn short_prefixes_are_a_200_empty_response_not_an_error() {
    for q in ["", "r", "%20"] {
        let (status, _, body) = get(seeded(), &format!("/api/v1/suggestions?q={q}")).await;
        assert_eq!(status, StatusCode::OK, "q {q:?}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["suggestions"].as_array().unwrap().is_empty(), "q {q:?}");
    }
    // A prefix with no corpus match is also a normal empty answer.
    let (status, _, body) = get(seeded(), "/api/v1/suggestions?q=zzz").await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["suggestions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn limits_clamp_into_the_documented_range() {
    let state = seeded();
    let (_, _, body) = get(state.clone(), "/api/v1/suggestions?q=ru&limit=0").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["suggestions"].as_array().unwrap().len(), 1);

    let (_, _, body) = get(state.clone(), "/api/v1/suggestions?q=ru&limit=999").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["suggestions"].as_array().unwrap().len() <= 10);

    // Garbage falls back to the default, never a 500.
    let (_, _, body) = get(state, "/api/v1/suggestions?q=ru&limit=plenty").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!v["suggestions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn scopes_filter_the_dictionaries_and_unknown_scopes_add_a_notice() {
    let state = seeded();
    let (_, _, body) = get(state.clone(), "/api/v1/suggestions?q=tok&scope=titles").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let items = v["suggestions"].as_array().unwrap();
    assert!(!items.is_empty(), "the corpus has a tokio-titled document");
    assert!(
        items.iter().all(|i| i["kind"] == "titles"),
        "titles scope returns only titles: {items:?}"
    );

    let (_, _, body) = get(state.clone(), "/api/v1/suggestions?q=tok&scope=terms").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let items = v["suggestions"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().all(|i| i["kind"] == "terms"));

    let (_, _, body) = get(state.clone(), "/api/v1/suggestions?q=tok&scope=banana").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["meta"]["notice"], "unknown scope; treated as all");
    assert!(v["suggestions"].as_array().unwrap().len() >= 2); // term + title

    let (_, _, body) = get(state, "/api/v1/suggestions?q=tok&scope=related").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["suggestions"].as_array().unwrap().is_empty());
    assert!(v["meta"]["notice"].is_string());
}

#[tokio::test]
async fn the_interface_language_is_echoed() {
    let state = seeded();
    // Explicit parameter wins over the header.
    let (_, _, body) = get_with_header(
        state.clone(),
        "/api/v1/suggestions?q=ru&interface_lang=de",
        Some("fr-CH, fr;q=0.9"),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["query"]["language"], "de");

    // Otherwise the first Accept-Language entry.
    let (_, _, body) =
        get_with_header(state, "/api/v1/suggestions?q=ru", Some("fr-CH, fr;q=0.9")).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["query"]["language"], "fr-ch");

    // Absent everywhere: no language field.
    let (_, _, body) = get(seeded(), "/api/v1/suggestions?q=ru").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["query"].get("language").is_none());
}

#[tokio::test]
async fn long_prefixes_are_truncated_at_the_documented_bound() {
    let long = format!("ru{}", "x".repeat(150));
    let q = format!("/api/v1/suggestions?q={long}");
    let (status, _, body) = get(seeded(), &q).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["query"]["prefix"].as_str().unwrap().len(), 128);
}

#[tokio::test]
async fn responses_are_identical_for_every_user() {
    // The privacy-critical property: no per-user state. Two fresh states produce the
    // same completion set (deterministic ranking over the same corpus).
    let (_, _, a) = get(seeded(), "/api/v1/suggestions?q=ru").await;
    let (_, _, b) = get(seeded(), "/api/v1/suggestions?q=ru").await;
    let va: serde_json::Value = serde_json::from_str(&a).unwrap();
    let vb: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(va["suggestions"], vb["suggestions"]);
}
