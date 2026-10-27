//! Full-loop smoke test: three services over real HTTP, one loop.
//!
//! ```text
//! lynx-api router (the search plane)   ← what users hit
//!   ↕ AnswerClient → POST /v1/answer
//! answer plane (real lynx_ai::engine)
//!   ↕ OpenAiCompatibleModel → POST /chat/completions
//! fixture model origin                 ← plays the pinned model
//! ```
//!
//! Nothing here mocks our own types: the answer plane is the real engine with
//! the real OpenAI-compatible provider, the search plane is the real router
//! with a real Tantivy generation on disk, and only the model origin is a
//! canned HTTP service — which is exactly what it is in any deployment
//! (vLLM, Ollama, llama.cpp).
//!
//! Properties under test:
//! - a search merges a cited, verified answer end to end;
//! - the identical search is a `from_cache` hit (three processes, one number);
//! - a model that ignores grounding fails closed into SERP-only 200.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use lynx_ai::controls::{AnswerCache, CircuitBreaker, KillSwitch, TokenBudget};
use lynx_ai::engine::{AnswerEngine, AnswerOutcome, AnswerRequest};
use lynx_ai::provider::{ModelProvider, OpenAiCompatibleModel};
use lynx_api::answer::AnswerClient;
use lynx_api::state::AppState;
use time::macros::datetime;
use tower::ServiceExt;

/// URL dialing prefixes (`/v1` after the model endpoint, none after the answer).
struct Service {
    dial: String,
    _guard: tokio::task::JoinHandle<()>,
}

fn grounded_content(behaviour: &str) -> serde_json::Value {
    if behaviour == "hallucinating" {
        // Plays a model that ignores grounding: the engine must refuse.
        serde_json::json!({ "text": "Invented conclusion lacking any citation marker at all." })
    } else {
        serde_json::json!({
            "text": "Ownership rules and the borrow checker keep Rust programs free of data races. [1]"
        })
    }
}

/// The pinned-model double: same body and usage accounting, every time.
async fn spawn_model(behaviour: &'static str) -> Service {
    let behaviour_router = Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(
            move |State(behaviour): State<&'static str>, _body: Json<serde_json::Value>| async move {
                let content = grounded_content(behaviour);
                Json(serde_json::json!({
                    "id": "chatcmpl-smoke",
                    "model": "m-1",
                    "choices": [{ "index": 0,
                                  "message": { "role": "assistant",
                                               "content": content.to_string() } }],
                    "usage": { "prompt_tokens": 21, "completion_tokens": 14 }
                }))
            }
        )
        .with_state(behaviour),
    );
    let (addr, guard) = spawn_axum(behaviour_router).await;
    Service {
        dial: format!("http://{addr}/v1"),
        _guard: guard,
    }
}

/// The real answer engine behind real HTTP, exactly the surface the binary
/// publishes (services/ai/src/bin/lynx-ai.rs).
async fn spawn_answer_plane(model: &Service) -> Service {
    let engine = Arc::new(AnswerEngine::new(
        ModelProvider::OpenAiCompatible(OpenAiCompatibleModel::new(&model.dial, "m-1")),
        Arc::new(KillSwitch::new()),
        Arc::new(TokenBudget::new(100_000)),
        Arc::new(CircuitBreaker::new(3, 60)),
        Arc::new(AnswerCache::new(600)),
        10,
        4_000,
        256,
    ));
    let app = Router::new().route(
        "/v1/answer",
        axum::routing::post(answer_the_serp).with_state(engine),
    );
    let (addr, guard) = spawn_axum(app).await;
    Service {
        dial: format!("http://{addr}"),
        _guard: guard,
    }
}

/// POST /v1/answer: the binary's handler as a route, calling the real engine.
async fn answer_the_serp(
    State(engine): State<Arc<AnswerEngine>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let question = body["question"].as_str().unwrap_or("").to_string();
    let documents: Vec<lynx_ai::sources::RankedDocument> = body["documents"]
        .as_array()
        .map(|docs| {
            docs.iter()
                .map(|d| lynx_ai::sources::RankedDocument {
                    doc_id: d["doc_id"].as_str().unwrap_or("").to_string(),
                    url: d["url"].as_str().unwrap_or("").to_string(),
                    title: d["title"].as_str().unwrap_or("").to_string(),
                    text: d["text"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let outcome = engine
        .answer(
            &lynx_common::time::SystemClock,
            &AnswerRequest {
                question,
                documents,
            },
        )
        .await;
    let value = match outcome {
        AnswerOutcome::Answer(answer) => serde_json::json!({
            "answered": true,
            "text": answer.text,
            "citations": answer.citations.iter().map(|c| {
                serde_json::json!({"ordinal": c.ordinal, "url": c.url, "doc_id": c.doc_id})
            }).collect::<Vec<_>>(),
            "grounded_doc_ids": answer.grounded_doc_ids,
            "from_cache": answer.from_cache
        }),
        AnswerOutcome::Refused(_) => {
            serde_json::json!({ "answered": false, "reason": "refused" })
        }
    };
    Json(value)
}

async fn spawn_axum(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let guard = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, guard)
}

/// A sealed index generation on disk — the real search surface.
fn sealed_index(dir: &Path) -> lynx_indexer::TantivyIndexReader {
    use lynx_indexer::IndexGeneration;
    let doc = lynx_indexer::IndexedDocument::new(
        lynx_common::ids::DocumentId::from_canonical_url("https://tokio.rs/doc/ownership"),
        "https://tokio.rs/doc/ownership",
        "Understanding ownership",
        "Ownership rules and the borrow checker keep Rust programs free of data races. \
         Rust checks memory safety at compile time.",
    );
    lynx_indexer::TantivyIndexReader::write_to_dir(
        dir,
        [doc],
        &IndexGeneration {
            id: 1,
            tokenizer_version: "lynx-test".to_string(),
            ranking_config_version: "v0.1.0".to_string(),
            created_at: datetime!(2026-03-01 00:00 UTC),
            doc_count: 1,
            size_bytes: 0,
            checksum: String::new(),
            shards: 1,
        },
    )
    .unwrap();
    lynx_indexer::TantivyIndexReader::open(dir).unwrap()
}

async fn get(app: Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
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

/// The whole loop: sealed index → verifier-backed answer plane → model origin.
async fn smoke_stack(behaviour: &'static str) -> (Router, Vec<tokio::task::JoinHandle<()>>) {
    let model = spawn_model(behaviour).await;
    let plane = spawn_answer_plane(&model).await;
    let client = AnswerClient::new(&plane.dial, 5_000).unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let dir = std::path::PathBuf::from(index_dir.path().as_os_str());
    let reader = sealed_index(&dir);
    let state = Arc::new(AppState {
        reader: Arc::new(reader),
        ranker: lynx_ranker::Ranker::default(),
        clock: None,
        started_at: datetime!(2026-03-01 00:00 UTC),
        security: lynx_api::middleware::SecurityContext::disabled(),
        answer: Some(Arc::new(client)),
        answer_max_sources: 10,
        answer_min_trust: 0.0,
    });
    (
        lynx_api::router::router(state),
        vec![model._guard, plane._guard],
    )
}

#[tokio::test]
async fn a_search_answers_with_citations_through_three_real_services() {
    let (app, _guards) = smoke_stack("grounded").await;

    let (status, headers, body) = get(app, "/api/v1/search?q=ownership").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let answer = &json["answer"];
    assert_eq!(
        answer["text"],
        "Ownership rules and the borrow checker keep Rust programs free of data races. [1]"
    );
    assert_eq!(answer["from_cache"], false);
    // Provenance points into the real index: the URL the generation holds.
    assert_eq!(
        answer["citations"][0]["url"],
        "https://tokio.rs/doc/ownership"
    );
    // The SERP rides along unchanged.
    assert!(json["results"].as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn an_identical_search_hits_the_answer_cache_and_skips_the_model() {
    let (app, _guards) = smoke_stack("grounded").await;

    let (_, _, first) = get(app.clone(), "/api/v1/search?q=ownership").await;
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["answer"]["from_cache"], false);

    let (_, _, second) = get(app, "/api/v1/search?q=ownership").await;
    let second: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_eq!(second["answer"]["from_cache"], true);
    assert_eq!(first["answer"]["text"], second["answer"]["text"]);
}

#[tokio::test]
async fn a_model_that_ignores_grounding_yields_serp_only() {
    let (app, _guards) = smoke_stack("hallucinating").await;
    let (status, _, body) = get(app, "/api/v1/search?q=ownership").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a grounded-answer failure is never the user's error"
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("answer").is_none(),
        "fail-closed removes the key entirely: {json}"
    );
}
