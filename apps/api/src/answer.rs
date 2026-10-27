//! The answer-plane client: how the search plane asks `lynx-ai` for an answer.
//!
//! Architecture (spec §Q): the answer plane is a separate process with exactly one
//! network attachment (the model endpoint). The search plane never loads a model;
//! it dials the answer plane over the internal network with the SERP head it already
//! ranked, and merges the verified answer into the response.
//!
//! Contract for a search that wants answers: **the SERP is never hostage to the
//! answer plane.** Every failure mode — connection refused, timeout, malformed
//! body, refusal slug, an unparseable response — degrades to "render the SERP
//! without an answer". There is no code path where the answer plane turns a search
//! into a 5xx, and no code path where it doubles latency beyond the configured
//! timeout.
//!
//! Privacy note (docs/privacy/): the request carries the active query text and the
//! top-ranked corpus snippets across a first-party hop. That is why `answer_service`
//! must be `https` in production (`lynx-config` validation), and why the field is
//! empty (feature off) in the shipped default config.

use serde::{Deserialize, Serialize};

/// One ranked document as the answer plane expects it — the same shape the
/// `lynx-ai` `/v1/answer` contract defines (`services/ai/src/bin/lynx-ai.rs`).
#[derive(Debug, Clone, Serialize)]
pub struct AnswerDocHead {
    /// Stable document id.
    pub doc_id: String,
    /// Canonical URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Stored body text used for grounding.
    pub text: String,
}

/// A verified answer merged into the search response (`docs/api/search.md`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnswerBlock {
    /// The verified summary: every sentence carries a valid citation.
    pub text: String,
    /// Citations in first-cited order.
    pub citations: Vec<AnswerCitation>,
    /// True when the answer plane served this from its cache.
    pub from_cache: bool,
}

/// A citation target rendered as provenance to the user.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnswerCitation {
    /// 1-based source ordinal as `[n]` in the answer text.
    pub ordinal: usize,
    /// Linked document URL.
    pub url: String,
    /// Document id.
    pub doc_id: String,
}

#[derive(Debug, Serialize)]
struct AnswerRequest<'a> {
    question: &'a str,
    documents: &'a [AnswerDocHead],
}

#[derive(Debug, Deserialize)]
struct AnswerResponse {
    answered: bool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    citations: Option<Vec<AnswerResponseCitation>>,
    #[serde(default)]
    from_cache: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AnswerResponseCitation {
    ordinal: usize,
    url: String,
    doc_id: String,
}

/// The dial-side client for the answer plane. Cloneable, cheap: it wraps a
/// connection pool.
#[derive(Debug, Clone)]
pub struct AnswerClient {
    /// Base URL, e.g. `https://ai-internal:8443` (no trailing slash required).
    base: String,
    /// Per-request deadline in milliseconds; the whole request is cut at this.
    timeout_ms: u64,
    http: reqwest::Client,
}

impl AnswerClient {
    /// Build a client for one answer-service base URL.
    ///
    /// # Errors
    /// [`std::time::Duration`] overflow is the only realistic failure; the pool
    /// build is delegated to reqwest.
    pub fn new(base: &str, timeout_ms: u64) -> Result<Self, reqwest::Error> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            timeout_ms,
            http: reqwest::Client::builder()
                .user_agent(concat!("lynx-api/", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }

    /// Ask the answer plane for one answer. `None` for any failure — timeout,
    /// transport, quota, refusal, malformed body — the caller renders the SERP.
    pub async fn answer(&self, question: &str, documents: &[AnswerDocHead]) -> Option<AnswerBlock> {
        if question.trim().is_empty() || documents.is_empty() {
            return None;
        }
        let deadline = std::time::Duration::from_millis(self.timeout_ms);
        let body = serde_json::to_vec(&AnswerRequest {
            question,
            documents,
        })
        .ok()?;
        let raw = self
            .http
            .post(format!("{}/v1/answer", self.base))
            .timeout(deadline)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .bytes()
            .await
            .ok()?;
        let response: AnswerResponse = serde_json::from_slice(&raw).ok()?;
        if !response.answered {
            // A typed refusal (`killed`, `budget_exhausted`, …) is a normal product
            // state: the SERP renders, `search.meta` says nothing happened. The
            // refusal slug is not user-relevant feedback and is not propagated.
            return None;
        }
        let citations = response
            .citations?
            .into_iter()
            .map(|c| AnswerCitation {
                ordinal: c.ordinal,
                url: c.url,
                doc_id: c.doc_id,
            })
            .collect();
        Some(AnswerBlock {
            text: response.text?,
            citations,
            from_cache: response.from_cache.unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::Json;

    async fn spawn_service(mode: Mode) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/v1/answer",
            axum::routing::post(
                move |State(mode): State<Mode>, _body: Json<serde_json::Value>| async move {
                    let value = match mode {
                        Mode::Answered => serde_json::json!({
                            "answered": true,
                            "text": "Ownership keeps Rust safe. [1]",
                            "citations": [
                                {"ordinal": 1, "url": "https://ok.test/", "doc_id": "d1"}
                            ],
                            "grounded_doc_ids": ["d1"],
                            "from_cache": false
                        }),
                        Mode::Refused => {
                            serde_json::json!({"answered": false, "reason": "budget_exhausted"})
                        }
                        Mode::Malformed => serde_json::json!({"weird": [1]}),
                        Mode::Slow => {
                            tokio::time::sleep(std::time::Duration::from_millis(2_000)).await;
                            serde_json::json!({"answered": true})
                        }
                    };
                    Json(value).into_response()
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app.with_state(mode)).await;
        });
        (addr, handle)
    }

    #[derive(Debug, Clone, Copy)]
    enum Mode {
        Answered,
        Refused,
        Malformed,
        Slow,
    }

    fn head() -> Vec<AnswerDocHead> {
        vec![AnswerDocHead {
            doc_id: "d1".into(),
            url: "https://ok.test/".into(),
            title: "t".into(),
            text: "Ownership keeps rust safe.".into(),
        }]
    }

    async fn client_for(
        mode: Mode,
        timeout_ms: u64,
    ) -> (AnswerClient, tokio::task::JoinHandle<()>) {
        let (addr, handle) = spawn_service(mode).await;
        (
            AnswerClient::new(&format!("http://{addr}"), timeout_ms).unwrap(),
            handle,
        )
    }

    #[tokio::test]
    async fn an_answered_response_is_parsed() {
        let (client, handle) = client_for(Mode::Answered, 5_000).await;
        let block = client.answer("q", &head()).await;
        let block = block.expect("answered");
        assert_eq!(block.text, "Ownership keeps Rust safe. [1]");
        assert_eq!(block.citations.len(), 1);
        assert!(!block.from_cache);
        handle.abort();
    }

    #[tokio::test]
    async fn a_refusal_degrades_to_none() {
        let (client, handle) = client_for(Mode::Refused, 5_000).await;
        assert_eq!(client.answer("q", &head()).await, None);
        handle.abort();
    }

    #[tokio::test]
    async fn a_malformed_body_degrades_to_none() {
        let (client, handle) = client_for(Mode::Malformed, 5_000).await;
        assert_eq!(client.answer("q", &head()).await, None);
        handle.abort();
    }

    #[tokio::test]
    async fn a_timeout_degrades_to_none() {
        let (client, handle) = client_for(Mode::Slow, 1).await;
        let started = std::time::Instant::now();
        assert_eq!(client.answer("q", &head()).await, None);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1_000),
            "timeout bounds the wait: {:?}",
            started.elapsed()
        );
        handle.abort();
    }

    #[tokio::test]
    async fn an_empty_head_or_question_never_dials() {
        let (client, handle) = client_for(Mode::Answered, 5_000).await;
        assert_eq!(client.answer("q", &[]).await, None);
        assert_eq!(client.answer("   ", &head()).await, None);
        handle.abort();
    }
}
