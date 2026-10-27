//! The `lynx-ai` binary — the answer plane's process (spec §Q, §Z step 18).
//!
//! Network discipline: this process is the *only* component that talks to the
//! model endpoint, and it talks to exactly one origin — the configured
//! `ai.endpoint`. The search plane stays network-free of model traffic by
//! construction: it never calls this library's provider; it calls this
//! process over loopback/Kubernetes-internal HTTP.
//!
//! ```text
//! POST /v1/answer   { "question": "...", "documents": [...] }
//! → 200 { "answered": true,  "text": "...", "citations": [...], ... }
//! → 200 { "answered": false, "reason": "budget_exhausted" }
//! (failures are refusals; nothing here turns into a 5xx unless malformed)
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use lynx_ai::controls::{AnswerCache, CircuitBreaker, KillSwitch, TokenBudget};
use lynx_ai::engine::{AnswerEngine, AnswerOutcome, AnswerRequest};
use lynx_ai::provider::{ModelProvider, OpenAiCompatibleModel};
use lynx_common::time::SystemClock;
use lynx_config::Config;

/// The request body: the SERP head computed by the search plane.
#[derive(Debug, Clone, Deserialize)]
pub struct AnswerBody {
    /// The active query.
    pub question: String,
    /// Already-ranked, deduplicated documents.
    pub documents: Vec<DocumentBody>,
}

/// One ranked document as the search plane sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentBody {
    /// Stable document id.
    pub doc_id: String,
    /// Canonical URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Snippet/body excerpt to ground from.
    pub text: String,
}

/// The response body: an answered summary or a typed refusal.
#[derive(Debug, Clone, Serialize)]
pub struct AnswerResponse {
    /// Whether an answer is available (false ⇒ render the SERP alone).
    pub answered: bool,
    /// Verified summary text (when answered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Structured citations (when answered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<CitationBody>>,
    /// The doc ids the summary was grounded in (when answered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounded_doc_ids: Option<Vec<String>>,
    /// Cache hit (when answered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_cache: Option<bool>,
    /// Refusal reason (when not answered): `killed` | `budget_exhausted` |
    /// `circuit_open` | `no_sources` | `unreliable_summary` | `empty_output` |
    /// `provider_error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A citation rendered to clients.
#[derive(Debug, Clone, Serialize)]
pub struct CitationBody {
    /// 1-based source ordinal.
    pub ordinal: usize,
    /// Indexed URL.
    pub url: String,
    /// Document id.
    pub doc_id: String,
}

/// Shared application state.
#[derive(Debug, Clone)]
pub struct AppState {
    engine: Arc<AnswerEngine>,
}

async fn answer(
    State(state): State<AppState>,
    Json(body): Json<AnswerBody>,
) -> (StatusCode, Json<AnswerResponse>) {
    let request = AnswerRequest {
        question: body.question,
        documents: body
            .documents
            .into_iter()
            .map(|d| lynx_ai::sources::RankedDocument {
                doc_id: d.doc_id,
                url: d.url,
                title: d.title,
                text: d.text,
            })
            .collect(),
    };
    let outcome = state.engine.answer(&SystemClock, &request).await;
    let response = match outcome {
        AnswerOutcome::Answer(answer) => AnswerResponse {
            answered: true,
            text: Some(answer.text),
            citations: Some(
                answer
                    .citations
                    .into_iter()
                    .map(|c| CitationBody {
                        ordinal: c.ordinal,
                        url: c.url,
                        doc_id: c.doc_id,
                    })
                    .collect(),
            ),
            grounded_doc_ids: Some(answer.grounded_doc_ids),
            from_cache: Some(answer.from_cache),
            reason: None,
        },
        AnswerOutcome::Refused(refusal) => AnswerResponse {
            answered: false,
            text: None,
            citations: None,
            grounded_doc_ids: None,
            from_cache: None,
            reason: Some(refusal_slug(&refusal)),
        },
    };
    (StatusCode::OK, Json(response))
}

/// Coarse refusal slugs for clients; never model text, never query text.
#[must_use]
fn refusal_slug(refusal: &lynx_ai::engine::AnswerRefusal) -> String {
    use lynx_ai::engine::AnswerRefusal as R;
    match refusal {
        R::Killed(_) => "killed".to_string(),
        R::BudgetExhausted { .. } => "budget_exhausted".to_string(),
        R::CircuitOpen => "circuit_open".to_string(),
        R::NoSources => "no_sources".to_string(),
        R::UnreliableSummary => "unreliable_summary".to_string(),
        R::EmptyOutput => "empty_output".to_string(),
        R::Provider(_) => "provider_error".to_string(),
    }
}

/// The health endpoint the compose healthchecks can use.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": lynx_ai::SERVICE_NAME }))
}

/// Build the router (exposed for tests).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/answer", post(answer))
        .route("/v1/health", axum::routing::get(health))
        .with_state(state)
}

/// Build the engine from a validated config.
///
/// # Errors
/// A config that enables AI with a provider other than the two this binary
/// serves (or `enabled` with `provider = "none"`).
pub fn engine_from_config(config: &Config) -> Result<AnswerEngine, String> {
    if !config.ai.enabled {
        return Err("ai.enabled is false; the answer binary has nothing to do".to_string());
    }
    let provider = match config.ai.provider.as_str() {
        "openai-compatible" | "self-hosted" => ModelProvider::OpenAiCompatible(
            OpenAiCompatibleModel::new(config.ai.endpoint.clone(), config.ai.model.clone()),
        ),
        other => {
            return Err(format!(
                "ai.provider = '{other}' is not servable by lynx-ai; use \
                 openai-compatible or self-hosted"
            ))
        }
    };
    Ok(AnswerEngine::new(
        provider,
        Arc::new(KillSwitch::new()),
        Arc::new(TokenBudget::new(config.ai.daily_token_budget)),
        Arc::new(CircuitBreaker::new(3, 60)),
        Arc::new(AnswerCache::new(
            i64::try_from(config.ai.answer_cache_ttl).unwrap_or(600),
        )),
        usize::try_from(config.ai.max_sources).unwrap_or(10),
        config.ai.max_source_tokens,
        1_024,
    ))
}

/// The process-wide config, loaded once at startup.
/// Load and validate the layered TOML configuration.
///
/// # Errors
/// Missing/unreadable files, parse errors, or environment-validation issues.
pub fn load_config() -> Result<lynx_config::Config, String> {
    let base = std::fs::read_to_string("config/example.toml")
        .map_err(|e| format!("cannot read config/example.toml: {e}"))?;
    let profile = std::env::var("LYNX_PROFILE").unwrap_or_else(|_| "development".to_string());
    let profile_path = format!("config/{profile}.toml");
    let layer = std::fs::read_to_string(&profile_path)
        .map_err(|e| format!("cannot read {profile_path}: {e}"))?;
    let config = lynx_config::Config::from_toml_layers(&[base.as_str(), layer.as_str()])
        .map_err(|e| format!("invalid configuration: {e}"))?;
    if let Err(issues) = lynx_config::validate::validate(&config) {
        return Err(format!(
            "configuration rejected: {}",
            issues
                .iter()
                .map(|i| format!("{}: {}", i.field, i.message))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok(config)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("lynx-ai: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let engine = match engine_from_config(&config) {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("lynx-ai: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // The answer process has its own bind courtesy of `ai.service_bind`; it must
    // never share the public API's address.
    let bind: SocketAddr = config
        .ai
        .service_bind
        .parse()
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 8180)));
    let app = router(AppState {
        engine: Arc::new(engine),
    });
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("lynx-ai: cannot bind {bind}: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!("lynx-ai listening on {bind}");
    match axum::serve(listener, app).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lynx-ai: server error: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn dev_engine() -> (Arc<AnswerEngine>, lynx_common::time::FixedClock) {
        // Tests drive the engine directly with a mock; the HTTP layer is
        // exercised below against an engine built by hand for determinism.
        let mut mock = lynx_ai::provider::MockProvider::new("test-model");
        mock.scripted = None;
        (
            Arc::new(AnswerEngine::new(
                ModelProvider::Mock(mock),
                Arc::new(KillSwitch::new()),
                Arc::new(TokenBudget::new(10_000)),
                Arc::new(CircuitBreaker::new(3, 60)),
                Arc::new(AnswerCache::new(600)),
                10,
                4_000,
                512,
            )),
            lynx_common::time::FixedClock::at(time::OffsetDateTime::UNIX_EPOCH),
        )
    }

    #[tokio::test]
    async fn the_http_surface_answers_through_the_engine() {
        // The mock provider is not reachable through engine_from_config, so
        // build the state manually: HTTP shape is what is under test here.
        let (engine, _clock) = dev_engine();
        let app = router(AppState { engine });
        let body = serde_json::json!({
            "question": "What keeps rust safe?",
            "documents": [{
                "doc_id": "d1",
                "url": "https://s1.test/",
                "title": "S1",
                "text": "Memory safety comes from ownership rules."
            }, {
                "doc_id": "d2",
                "url": "https://s2.test/",
                "title": "S2",
                "text": "The borrow checker enforces the rules."
            }]
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/v1/answer")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["answered"], true);
        assert!(v["text"].as_str().unwrap().contains("[1]"));
        assert_eq!(v["citations"][0]["doc_id"], "d1");

        // And the second identical request is a cache hit with looser budget
        // (determinism pinned by the engine tests; the HTTP layer just pipes).
        let response = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/v1/answer")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["answered"], true);
        assert_eq!(v["from_cache"], true);
    }

    #[tokio::test]
    async fn malformed_bodies_are_4xx_not_5xx() {
        let (engine, _clock) = dev_engine();
        let app = router(AppState { engine });
        let response = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/v1/answer")
                    .header("content-type", "application/json")
                    .body(Body::from("{\"unclosed\""))
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum's default Json rejection is 400 BAD_REQUEST; anything but 5xx
        // is the contract (malformed input can never crash the plane).
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn health_carries_the_service_name() {
        let (engine, _clock) = dev_engine();
        let app = router(AppState { engine });
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["service"], "ai");
    }
}
