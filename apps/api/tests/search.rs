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
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: None,
        answer_max_sources: 10,
        answer_min_trust: 0.0,
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

/// Build a state with a fresh index holding a single document.
fn state_with(doc: IndexedDocument) -> Arc<AppState> {
    let mut index = InMemoryIndex::new();
    index.add(doc);
    Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
        started_at: datetime!(2026-02-18 00:00 UTC),
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: None,
        answer_max_sources: 10,
        answer_min_trust: 0.0,
    })
}

#[tokio::test]
async fn negation_penalises_without_excluding() {
    let mut clean = IndexedDocument::new(
        DocumentId::from_canonical_url("https://a.test/"),
        "https://a.test/",
        "Rust programming",
        "A systems language.",
    );
    clean.lang = Some("en".to_string());
    let mut negated = IndexedDocument::new(
        DocumentId::from_canonical_url("https://b.test/"),
        "https://b.test/",
        "Rust programming",
        "A systems language that also mentions golang.",
    );
    negated.lang = Some("en".to_string());
    let mut index = InMemoryIndex::new();
    index.add(clean);
    index.add(negated);
    let state = Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
        started_at: datetime!(2026-02-18 00:00 UTC),
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: None,
        answer_max_sources: 10,
        answer_min_trust: 0.0,
    });

    let (_, _, body) = get_with(state, "/api/v1/search?q=rust+-golang&explain=1").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Both appear; the one containing the negated term is downranked.
    assert_eq!(v["meta"]["returned"], 2);
    assert_eq!(v["results"][0]["url"], "https://a.test/");
    assert_eq!(
        v["results"][0]["signals"]["negation"].as_f64().unwrap(),
        0.0
    );
    assert!(v["results"][1]["signals"]["negation"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn filetype_and_ext_filter_results() {
    let mut index = InMemoryIndex::new();
    let mut pdf = IndexedDocument::new(
        DocumentId::from_canonical_url("https://a.test/guide.pdf"),
        "https://a.test/guide.pdf",
        "Rust guide",
        "Rust systems programming",
    );
    pdf.content_type = Some("application/pdf".to_string());
    let mut html = IndexedDocument::new(
        DocumentId::from_canonical_url("https://b.test/"),
        "https://b.test/",
        "Rust guide",
        "Rust systems programming",
    );
    html.content_type = Some("text/html".to_string());
    index.add(pdf);
    index.add(html);
    let state = Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
        started_at: datetime!(2026-02-18 00:00 UTC),
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: None,
        answer_max_sources: 10,
        answer_min_trust: 0.0,
    });
    let (_, _, b1) = get_with(state.clone(), "/api/v1/search?q=rust+filetype:pdf").await;
    let v: serde_json::Value = serde_json::from_str(&b1).unwrap();
    assert_eq!(v["meta"]["returned"], 1);
    assert_eq!(v["results"][0]["url"], "https://a.test/guide.pdf");
    let (_, _, b2) = get_with(state, "/api/v1/search?q=rust+ext:pdf").await;
    let v: serde_json::Value = serde_json::from_str(&b2).unwrap();
    assert_eq!(v["meta"]["returned"], 1);
}

// ---------------------------------------------------------------------------
// Middleware: security headers, request ids, rate limiting, fallbacks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_search_response_carries_the_security_headers() {
    let (status, headers, _) = get("/api/v1/search?q=rust").await;
    assert_eq!(status, StatusCode::OK);
    // `docs/api/errors.md` HTTP semantics + spec §P: the set is asserted here so it
    // cannot be worn away silently.
    for (name, expected) in [
        ("content-security-policy", "default-src 'none'"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        ("vary", "Accept, Accept-Language"),
    ] {
        let got = headers
            .get(name)
            .unwrap_or_else(|| panic!("missing header {name}"))
            .to_str()
            .unwrap();
        assert!(got.starts_with(expected), "{name} was {got:?}");
    }
    assert!(headers.get("x-request-id").is_some(), "request id present");
}

#[tokio::test]
async fn status_keeps_its_cache_contract_and_still_gets_headers() {
    let (_, headers, _) = get("/api/v1/status").await;
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=60"
    );
    assert!(headers.get("content-security-policy").is_some());
    assert!(headers.get("x-request-id").is_some());
}

#[tokio::test]
async fn well_formed_request_id_is_echoed_and_garbage_is_replaced() {
    let valid = lynx_logging::RequestId::new().to_string();
    let (_, headers, _) = get_with_header(seeded(), "x-request-id", &valid).await;
    assert_eq!(
        headers.get("x-request-id").map(|v| v.to_str().unwrap()),
        Some(valid.as_str())
    );

    let (_, headers, _) =
        get_with_header(seeded(), "x-request-id", "<script>alert(1)</script>").await;
    let echoed = headers.get("x-request-id").unwrap().to_str().unwrap();
    assert!(
        lynx_logging::RequestId::parse(echoed).is_some(),
        "garbage got fresh id"
    );
    assert_ne!(echoed, "<script>alert(1)</script>");
}

async fn get_with_header(
    state: Arc<AppState>,
    name: &str,
    value: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let app = lynx_api::router::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/search?q=rust")
                .header(name, value)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    collect(resp).await
}

async fn collect(resp: axum::response::Response) -> (StatusCode, axum::http::HeaderMap, String) {
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn rate_limit_blocks_the_third_request_with_the_documented_envelope() {
    let state = {
        let mut s = AppState::default();
        s.started_at = datetime!(2026-02-18 00:00 UTC);
        s.security = lynx_api::middleware::SecurityContext::with_limit(2);
        Arc::new(s)
    };
    let app = lynx_api::router::router(state.clone());
    let mut last = None;
    for _ in 0..3 {
        last = Some(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/v1/search?q=rust")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        );
    }
    let (status, headers, body) = collect(last.unwrap()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], "RATE_LIMITED");
    assert_eq!(v["error"]["retryable"], true);
    assert!(v["error"]["retry_after_ms"].as_u64().unwrap() > 0);
    assert_eq!(v["error"]["detail"]["limit"], 2);
    assert_eq!(v["error"]["detail"]["window_seconds"], 60);
    assert!(headers.get("retry-after").is_some());
    assert!(headers.get("x-request-id").is_some());
    assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "0");
}

#[tokio::test]
async fn unknown_paths_get_the_404_envelope_not_plain_text() {
    let (status, _, body) = get("/api/v1/unknown").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], "NOT_FOUND");
    assert_eq!(v["error"]["status"], 404);
}

#[tokio::test]
async fn hostile_url_schemes_render_as_text_not_links() {
    let mut evil = IndexedDocument::new(
        DocumentId::from_canonical_url("https://evil.test/"),
        "javascript:alert(1)",
        "Innocent title",
        "Some body text.",
    );
    evil.lang = Some("en".to_string());
    let (_, _, body) = get_with(state_with(evil), "/api/v1/search?q=innocent&format=html").await;
    // The escaped javascript: URL must never become an href.
    assert!(!body.contains(r#"href="javascript:"#));
    assert!(body.contains("alert(1)"), "url text still shown, escaped");
    assert!(body.contains("<span class=\"result-link\">"));
    assert!(!body.contains("<a class=\"result-link\""));
}

#[tokio::test]
async fn extreme_offset_does_not_panic() {
    let (_, _, body) = get("/api/v1/search?q=rust&offset=18446744073709551615").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Attempting to page past the offset cap yields no results rather than a panic.
    assert_eq!(v["meta"]["returned"], 0);
}

#[tokio::test]
async fn truncated_query_carries_a_notice() {
    let long_q = "rust ".repeat(200); // ~1000 bytes > the 512-byte cap
    let (_, _, body) = get(&format!("/api/v1/search?q={}", urlencode(&long_q))).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["meta"]["truncated"], true);
    assert!(v["meta"]["notice"].is_string(), "the notice explains it");
}

#[tokio::test]
async fn explain_defaults_off_and_opt_in_includes_signals() {
    let (_, _, body) = get("/api/v1/search?q=rust").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["results"][0]["signals"].is_null(), "signals are opt-in");

    let (_, _, body) = get("/api/v1/search?q=rust&explain=1").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["results"][0]["signals"].is_object());
}

#[tokio::test]
async fn malformed_parameters_fall_back_to_defaults() {
    // count=abc and non-UTF-8 bytes must not produce a 500 or a plain-text 400.
    let (status, headers, body) = get("/api/v1/search?q=rust&count=abc").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["meta"]["returned"], 1);
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

// ---------------------------------------------------------------------------
// §Z Step 17: no-store everywhere, Global Privacy Control
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_store_covers_every_error_path() {
    // 404 fallback and 405 method-not-allowed must not respsect anything that could
    // become cache-poisoning bait either: the header is checked on every
    // non-cacheable surface, error statuses included.
    for (label, request) in [
        (
            "404 fallback",
            Request::builder()
                .uri("/api/v1/nope")
                .body(Body::empty())
                .unwrap(),
        ),
        (
            "405 method",
            Request::builder()
                .method(axum::http::Method::POST)
                .uri("/api/v1/search")
                .body(Body::empty())
                .unwrap(),
        ),
    ] {
        let app = lynx_api::router::router(seeded());
        let (status, headers, _) = collect(app.oneshot(request).await.unwrap()).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "{label} must be an error to test the header path"
        );
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "no-store, no-cache, must-revalidate",
            "{label} must be no-store"
        );
    }
}

#[tokio::test]
async fn rate_limited_response_is_no_store() {
    // The 429 is constructed inside the rate-limit middleware, which sits *inside*
    // the security-headers layer, so the envelope still wears the full set.
    let state = {
        let mut s = AppState::default();
        s.started_at = datetime!(2026-02-18 00:00 UTC);
        s.security = lynx_api::middleware::SecurityContext::with_limit(1);
        Arc::new(s)
    };
    let app = lynx_api::router::router(state.clone());
    app.oneshot(
        Request::builder()
            .uri("/api/v1/search?q=rust")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    let (_, headers, _) = collect(
        lynx_api::router::router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search?q=rust")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        headers.get(header::CACHE_CONTROL).unwrap(),
        "no-store, no-cache, must-revalidate",
        "a shared cache must never hold a rate-limit body"
    );
}

#[tokio::test]
async fn gpc_clients_get_the_identical_body_and_no_cookies() {
    // Global Privacy Control: LYNX sets no cookies and does no tracking, so a GPC
    // signal is honoured by construction. This test pins the two visible facts:
    // the body is byte-identical to a non-GPC client's, and no Set-Cookie exists
    // on either path. The middleware strip is defence in depth for a future
    // cookie-setting regression (lynx_security::privacy documents the design).
    let normal = get("/api/v1/search?q=rust").await;
    let gpc = get_with_header(seeded(), "sec-gpc", "1").await;

    assert_eq!(gpc.0, normal.0);
    assert_eq!(gpc.2, normal.2, "GPC must not change the served content");
    assert!(
        normal.1.get(header::SET_COOKIE).is_none(),
        "a normal response already carries no cookie"
    );
    assert!(
        gpc.1.get(header::SET_COOKIE).is_none(),
        "a GPC response must never carry a cookie"
    );
}
