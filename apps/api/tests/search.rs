//! End-to-end tests for the search endpoint, through the real router.
//!
//! These exercise the HTTP surface (`GET /api/v1/search`) as a client sees it: status
//! codes, content negotiation, escaping, and the accessibility-relevant markup, rather
//! than calling the handler directly.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::fmt::Write as _;
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
    })
}

async fn get_with(state: Arc<AppState>, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let app = lynx_api::router::router(state);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn get(uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    get_with(seeded(), uri).await
}

#[tokio::test]
async fn json_is_the_default_representation() {
    let (status, headers, body) = get("/api/v1/search?q=rust").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["results"][0]["url"], "https://tokio.rs/");
    assert!(v["meta"]["index_generation"].is_string());
}

#[tokio::test]
async fn html_format_returns_a_usable_page() {
    let (status, headers, body) = get("/api/v1/search?q=rust&format=html").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(body.starts_with("<!DOCTYPE html>"));
    assert!(body.contains("<ol id=\"results\""));
    assert!(body.contains("Tokio"));
    // The result link's accessible name comes from the title text, not the URL.
    assert!(body.contains("class=\"result-link\""));
}

#[tokio::test]
async fn html_escapes_untrusted_text() {
    let (_, _, body) =
        get("/api/v1/search?q=%3Cscript%3Ealert(1)%3C%2Fscript%3E&format=html").await;
    assert!(!body.contains("<script>alert(1)</script>"));
    assert!(body.contains("&lt;script&gt;"));
}

#[tokio::test]
async fn missing_query_is_a_400_envelope() {
    let (status, _, body) = get("/api/v1/search?q=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], "INVALID_QUERY");
    assert_eq!(v["error"]["status"], 400);
    assert_eq!(v["error"]["retryable"], false);
}

#[tokio::test]
async fn empty_result_set_is_a_200_not_a_404() {
    let (status, _, body) = get("/api/v1/search?q=zzzznotaword&format=html").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("No results"));
}

#[tokio::test]
async fn html_has_accessibility_landmarks() {
    let (_, _, body) = get("/api/v1/search?q=rust&format=html").await;
    assert!(
        body.contains("aria-live=\"polite\""),
        "result count is announced"
    );
    assert!(
        body.contains("aria-busy=\"false\""),
        "results list has busy state"
    );
    assert!(
        body.contains("role=\"search\""),
        "search form is a landmark"
    );
    assert!(body.contains("Skip to results"), "skip link is present");
}

#[tokio::test]
async fn html_offers_operators_help_and_theme() {
    let (_, _, body) = get("/api/v1/search?q=rust&format=html").await;
    assert!(
        body.contains("Search operators"),
        "operators help is present"
    );
    assert!(
        body.contains("prefers-color-scheme"),
        "theme honours the system preference"
    );
    assert!(
        body.contains("prefers-contrast") && body.contains("prefers-reduced-motion"),
        "contrast and motion preferences are honoured"
    );
}

#[tokio::test]
async fn pagination_links_echo_the_offset() {
    let (_, _, body) = get("/api/v1/search?q=rust+OR+zig&count=1&offset=1&format=html").await;
    assert!(
        body.contains("rel=\"prev\""),
        "a previous link appears past page 1"
    );
    assert!(
        body.contains("offset=0"),
        "the previous link steps back by the page size"
    );
}

#[tokio::test]
async fn total_is_the_match_count_not_the_corpus_size() {
    // The corpus holds two documents; only one matches "rust".
    let (_, _, body) = get("/api/v1/search?q=rust").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["meta"]["total"], 1);
    assert_eq!(v["meta"]["returned"], 1);
}

#[tokio::test]
async fn operators_used_are_reported() {
    let (_, _, body) = get("/api/v1/search?q=rust+site:tokio.rs").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["query"]["operators_used"][0], "site");
}

#[tokio::test]
async fn cursor_walks_pages() {
    let (_, _, first) = get("/api/v1/search?q=rust+OR+zig&count=1").await;
    let v: serde_json::Value = serde_json::from_str(&first).unwrap();
    let first_url = v["results"][0]["url"].as_str().unwrap().to_string();
    let cursor = v["meta"]["next_cursor"]
        .as_str()
        .expect("first page has a cursor")
        .to_string();

    let (_, _, second) = get(&format!(
        "/api/v1/search?q=rust+OR+zig&count=1&cursor={}",
        urlencode(&cursor)
    ))
    .await;
    let v2: serde_json::Value = serde_json::from_str(&second).unwrap();
    let second_url = v2["results"][0]["url"].as_str().unwrap().to_string();
    assert_ne!(first_url, second_url, "the cursor advances the page");
    assert!(
        v2["meta"]["next_cursor"].is_null(),
        "the last page has no cursor"
    );
}

#[tokio::test]
async fn intitle_restricts_to_the_title() {
    // "zig" is only in the title of the zig document and nowhere else, so it matches.
    let (_, _, body) = get("/api/v1/search?q=intitle:tokio").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["results"][0]["url"], "https://tokio.rs/");
    assert_eq!(v["meta"]["returned"], 1);

    // Neither document has both: zig lacks "rust", tokio lacks "zig" in its title.
    let (_, _, none) = get("/api/v1/search?q=rust+intitle:zig").await;
    let v: serde_json::Value = serde_json::from_str(&none).unwrap();
    assert_eq!(v["meta"]["returned"], 0);
}

#[tokio::test]
async fn negation_penalises_without_excluding() {
    let mut index = InMemoryIndex::new();
    let mut clean = IndexedDocument::new(
        DocumentId::from_canonical_url("https://a.test/"),
        "https://a.test/",
        "Rust programming",
        "A systems language.",
    );
    clean.lang = Some("en".to_string());
    index.add(clean);
    let mut negated = IndexedDocument::new(
        DocumentId::from_canonical_url("https://b.test/"),
        "https://b.test/",
        "Rust programming",
        "A systems language that also mentions golang.",
    );
    negated.lang = Some("en".to_string());
    index.add(negated);

    let state = Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
        started_at: datetime!(2026-02-18 00:00 UTC),
    });

    let (_, _, body) = get_with(state, "/api/v1/search?q=rust+-golang").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Both appear; the one containing the negated term is downranked.
    assert_eq!(v["meta"]["returned"], 2);
    assert_eq!(v["results"][0]["url"], "https://a.test/");
    assert_eq!(v["results"][0]["signals"]["negation"].as_f64().unwrap(), 0.0);
    assert!(v["results"][1]["signals"]["negation"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn status_reports_index_health() {
    let (status, headers, body) = get("/api/v1/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "healthy");
    assert_eq!(v["index"]["documents"], 2);
    assert_eq!(v["quality"]["ranking_config_version"], "v0.1.0");
    assert!(v["service"]["version"].is_string());
    assert_eq!(v["service"]["degraded"], false);
}

/// Percent-encode a string for safe use in a query string.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
