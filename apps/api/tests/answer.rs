//! End-to-end tests for the answer merge (`/api/v1/search` × `lynx-ai`).
//!
//! The answer plane is a separate process; here a fixture HTTP service plays the
//! role with canned contract responses. The property under test: the search is
//! never hostage to the answer plane — an answer appears only when the answer
//! plane verifies one, and every other outcome (disabled, refused, down,
//! unreachable) renders the ordinary SERP.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use lynx_api::answer::AnswerClient;
use lynx_api::state::AppState;
use lynx_common::ids::DocumentId;
use lynx_indexer::{InMemoryIndex, IndexedDocument};
use lynx_ranker::Ranker;
use time::macros::datetime;
use tower::ServiceExt;

/// A fixture `lynx-ai` with configurable behaviour per call.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// A verified two-sentence answer with citations.
    Answered,
    /// A typed refusal (`budget_exhausted`), the normal product "no answer".
    Refused,
    /// No listening endpoint at all.
    Dark,
}

#[tokio::test]
async fn an_answered_search_carries_the_verified_answer() {
    let (client, _guard) = spawn_fixture(Mode::Answered).await;
    let state = seeded(Some(client));
    let (status, headers, body) =
        get_with(state.clone(), "/api/v1/search?format=json&q=rust").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    let answer = parse_field(&body, "answer");
    assert!(
        answer.contains("[1]"),
        "answer text carries its citation: {body}"
    );
    assert!(
        answer.contains("https://s1.test/"),
        "citations are rendered: {answer}"
    );

    // The HTML representation renders the answer as a cited block, above results.
    let (_, _, html) = get_with(state, "/api/v1/search?format=html&q=rust").await;
    assert!(
        html.contains("aria-label=\"Grounded answer\""),
        "answer block present"
    );
    assert!(
        html.contains("answer-cite"),
        "citation markers become links"
    );
    assert!(
        html.find("aria-label=\"Grounded answer\"").unwrap() < html.find("id=\"results\"").unwrap(),
        "the answer reads above the result list"
    );
}

#[tokio::test]
async fn a_refused_answer_never_shows_in_the_response() {
    // The refusal is a normal product state, communicated by absence: there is
    // no `answered: false` surface in the search response — the shape stays the
    // SERP, possibly with no answer key at all.
    let (client, _guard) = spawn_fixture(Mode::Refused).await;
    let state = seeded(Some(client));
    let (status, _, body) = get_with(state, "/api/v1/search?q=rust").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.contains("\"answer\""),
        "a refusal removes the key entirely: {body}"
    );
}

#[tokio::test]
async fn a_dark_answer_plane_degrades_to_serp_only() {
    // Connection refused is indistinguishable from any other "no answer".
    let (client, _guard) = spawn_fixture(Mode::Dark).await;
    let state = seeded(Some(client));
    let (status, _, body) = get_with(state, "/api/v1/search?q=rust").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("\"answer\""), ": {body}");
}

#[tokio::test]
async fn a_deployment_without_the_answer_plane_contacts_nobody() {
    let state = seeded(None);
    let (status, _, body) = get_with(state, "/api/v1/search?q=rust").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("\"answer\""), ": {body}");
}

// ── fixtures ──────────────────────────────────────────────────────────────────

fn seeded(client: Option<AnswerClient>) -> Arc<AppState> {
    let mut index = InMemoryIndex::new();
    let mut rust = IndexedDocument::new(
        DocumentId::from_canonical_url("https://s1.test/"),
        "https://s1.test/",
        "Rust borrow checking",
        "Rust prevents data races at compile time through ownership rules.",
    );
    rust.lang = Some("en".into());
    rust.published_at = Some(datetime!(2025-11-04 00:00 UTC));
    rust.quality_score = 0.9;
    index.add(rust);
    Arc::new(AppState {
        reader: Arc::new(index),
        ranker: Ranker::default(),
        clock: None,
        started_at: datetime!(2026-03-01 00:00 UTC),
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: client.map(Arc::new),
        answer_max_sources: 10,
    })
}

/// Spawn the fixture answer plane. Returns a client dialing it; `_guard` keeps
/// the server task alive for the connection's lifetime (aborted at test end).
async fn spawn_fixture(mode: Mode) -> (AnswerClient, tokio::task::JoinHandle<()>) {
    if mode == Mode::Dark {
        // Nothing binds: dialing this port is refused.
        let client = AnswerClient::new("http://127.0.0.1:1", 300).expect("client builds");
        let guard = tokio::spawn(async {});
        return (client, guard);
    }
    let app = axum::Router::new().route(
        "/v1/answer",
        axum::routing::post(
            move |State(mode): State<Mode>, _body: Json<serde_json::Value>| async move {
                let value = match mode {
                    Mode::Answered => serde_json::json!({
                        "answered": true,
                        "text": "Ownership keeps Rust safe. [1]",
                        "citations": [
                            {"ordinal": 1, "url": "https://s1.test/", "doc_id": "d1"}
                        ],
                        "grounded_doc_ids": ["d1"],
                        "from_cache": false
                    }),
                    Mode::Refused => {
                        serde_json::json!({"answered": false, "reason": "budget_exhausted"})
                    }
                    _ => unreachable!("handled outside"),
                };
                Json(value).into_response()
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let guard = tokio::spawn(async move {
        let _ = axum::serve(listener, app.with_state(mode)).await;
    });
    let client = AnswerClient::new(&format!("http://{addr}"), 2_000).expect("client builds");
    (client, guard)
}

async fn get_with(state: Arc<AppState>, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let app = lynx_api::router::router(state);
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

/// The value of `body["answer"]` as raw JSON text; asserts presence.
fn parse_field<'a>(body: &'a str, field: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    serde_json::to_string(&v[field])
        .unwrap()
        .trim_matches('"')
        .replace("\\\"", "\"")
}
