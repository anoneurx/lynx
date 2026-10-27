//! Model providers: one permit-listed endpoint, structured output, no tools.
//!
//! Two implementations ship in v0.1:
//!
//! * [`MockProvider`] — deterministic, offline; the tests pin *engine* promises
//!   (grounding, fail-closed, budgets) with canned outputs instead of a live
//!   model. It is also the fallback a deployment can pin for air-gapped
//!   smoke checks.
//! * [`OpenAiCompatibleModel`] — `/v1/chat/completions` against exactly the one
//!   configured endpoint (`ai.endpoint`), `temperature 0`, JSON response-objects
//!   only. Where "separate binary with network access limited to one model
//!   endpoint" (spec §Q) meets the real world.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ProviderError;

/// Zero temperature in the request: the summariser must not be creative.
pub const TEMPERATURE: f64 = 0.0;

/// Top-`p` per docs/ai/astra.md.
pub const TOP_P: f64 = 0.1;

/// A structured completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    /// The pinned model id from config (`ai.model`); never floating.
    pub model: String,
    /// System instructions (the compile-time prompt).
    pub system: String,
    /// The user message carrying the data channel.
    pub user: String,
    /// Maximum completion tokens to budget the response.
    pub max_tokens: u32,
}

/// A structured completion response: the text plus provider-reported usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The model's text output.
    pub text: String,
    /// Provider-reported prompt tokens (0 when the provider does not say).
    pub prompt_tokens: u64,
    /// Provider-reported completion tokens.
    pub completion_tokens: u64,
}

/// The provider surface the engine drives. An enum rather than a trait object —
/// the set of providers is closed by config (`none | openai-compatible |
/// self-hosted`) and static dispatch keeps the fail-open question moot: there is
/// no construction where "no provider" becomes "some provider".
#[derive(Debug, Clone)]
pub enum ModelProvider {
    /// The deterministic offline provider (tests and smoke checks).
    Mock(MockProvider),
    /// A `/v1/chat/completions`-compatible endpoint.
    OpenAiCompatible(OpenAiCompatibleModel),
}

impl ModelProvider {
    /// The pinned model id this provider will always use.
    #[must_use]
    pub fn model(&self) -> &str {
        match self {
            Self::Mock(mock) => &mock.model,
            Self::OpenAiCompatible(model) => &model.model,
        }
    }

    /// Run one completion.
    ///
    /// # Errors
    /// [`ProviderError`] for transport, schema, or refusals; the engine fails
    /// closed on every variant.
    pub async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        match self {
            Self::Mock(mock) => mock.complete(request),
            Self::OpenAiCompatible(model) => model.complete(request).await,
        }
    }
}

/// Deterministic offline provider.
///
/// It summarises by offering the *first sentences of each source passage with a
/// [N] citation*, optionally following a scripted behaviour so tests can pin
/// hallucination, invalid citations, and schema shapes without a live model.
#[derive(Debug, Clone)]
pub struct MockProvider {
    /// Pinned model id.
    pub model: String,
    /// An optional scripted response for the next call (test fixture).
    pub scripted: Option<String>,
}

impl MockProvider {
    /// A mock pinned to `model`.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            scripted: None,
        }
    }

    /// Extractive default: first sentence(s) per passage, cited.
    #[must_use]
    pub fn extractive_default(request: &CompletionRequest) -> CompletionResponse {
        // Pull the numbered sources back out of the user message's data channel.
        let mut source_text: Vec<(String, String)> = Vec::new();
        let mut rest = request.user.as_str();
        while let Some(start) = rest.find("<data-source") {
            let Some(open_end) = rest[start..].find(">\n") else {
                break;
            };
            let body_start = start + open_end + 2;
            let Some(close) = rest[body_start..].find("</data-source>") else {
                break;
            };
            let body = &rest[body_start..body_start + close];
            let trimmed = body.trim().to_string();
            let n = rest[start..body_start]
                .split("n=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("?")
                .to_string();
            source_text.push((n, trimmed));
            rest = &rest[body_start + close..];
        }
        let mut text = String::new();
        for (n, body) in &source_text {
            let first = body
                .split(". ")
                .next()
                .unwrap_or(body.as_str())
                .trim()
                .to_string();
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&first);
            text.push_str(". [");
            text.push_str(n.as_str());
            text.push(']');
        }
        if text.is_empty() {
            text.push_str("SOURCES_DO_NOT_SUPPORT_ANSWER");
        }
        CompletionResponse {
            completion_tokens: u64::from(crate::sources::tokens_estimate(&text)),
            prompt_tokens: u64::from(crate::sources::tokens_estimate(&request.user)),
            text,
        }
    }

    /// Sync complete (the mock has no I/O); matches [`ModelProvider::complete`].
    ///
    /// # Errors
    /// Never; the mock is total.
    pub fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        // The pinned model id is part of the contract even offline: a mock
        // receiving an unpinned id is a caller bug worth failing on.
        if request.model != self.model {
            return Err(ProviderError::model_mismatch(&request.model, &self.model));
        }
        let text = match &self.scripted {
            Some(scripted) => scripted.clone(),
            None => Self::extractive_default(request).text,
        };
        Ok(CompletionResponse {
            prompt_tokens: u64::from(crate::sources::tokens_estimate(&request.user)),
            completion_tokens: u64::from(crate::sources::tokens_estimate(&text)),
            text,
        })
    }
}

/// The OpenAI-compatible provider bound to one configured endpoint.
///
/// Construction is the *only* mutation: `temperature 0`, `top_p 0.1`, and
/// `response_format` of `{"type": "json_object"}` so the output is at least a
/// parseable JSON object, then schema-validated by the engine.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleModel {
    /// The configured endpoint origin, e.g. `https://ai.internal:8080/v1`.
    pub endpoint: String,
    /// The pinned model id.
    pub model: String,
}

impl OpenAiCompatibleModel {
    /// Bind to `endpoint` for `model`.
    #[must_use]
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
        }
    }

    /// Run one `/v1/chat/completions` call.
    ///
    /// # Errors
    /// [`ProviderError::Transport`] on HTTP failure, [`ProviderError::Schema`]
    /// when the response does not carry the expected shape, or a mapped 4xx.
    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        if request.model != self.model {
            return Err(ProviderError::model_mismatch(&request.model, &self.model));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| ProviderError::Transport("client build failed".to_string()))?;
        let body = Value::Object(
            [
                ("model", Value::String(self.model.clone())),
                ("temperature", serde_json::json!(TEMPERATURE)),
                ("top_p", serde_json::json!(TOP_P)),
                ("max_tokens", Value::Number(request.max_tokens.into())),
                (
                    "messages",
                    Value::Array(vec![
                        serde_json::json!({ "role": "system", "content": request.system }),
                        serde_json::json!({ "role": "user", "content": request.user }),
                    ]),
                ),
                ("response_format", json!({ "type": "json_object" })),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        );
        let response = client
            .post(format!("{}/chat/completions", self.endpoint))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&body).map_err(|e| ProviderError::Schema(e.to_string()))?)
            .send()
            .await
            .map_err(|e| ProviderError::Transport(e.to_string()))?;
        let status = response.status();
        let payload: Value = serde_json::from_slice(
            &response
                .bytes()
                .await
                .map_err(|e| ProviderError::Transport(format!("unreadable body: {e}")))?,
        )
        .map_err(|e| ProviderError::Transport(format!("unreadable body: {e}")))?;
        if !status.is_success() {
            return Err(ProviderError::Transport(format!(
                "HTTP {status}: {}",
                payload
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("(no message)")
            )));
        }
        let text = payload
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| ProviderError::schema("choices[0].message.content missing"))?;
        // response_format json_object: the content is the JSON string of
        // {"text": ...}. Unwrap the nesting so the engine sees one contract;
        // deployments that return the raw text unwrapped get wrapped here so
        // the schema contract stays: exactly one "text" field.
        let content: Value = serde_json::from_str(text).unwrap_or(json!({ "text": text }));
        let inner_text = content
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let usage = payload.get("usage").cloned().unwrap_or(Value::Null);
        Ok(CompletionResponse {
            prompt_tokens: usage
                .get("prompt_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(u64::from(crate::sources::tokens_estimate(&request.user))),
            completion_tokens: usage
                .get("completion_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            text: inner_text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::assemble;
    use crate::sources::{prepare_sources, RankedDocument};

    fn fixture_sources() -> Vec<RankedDocument> {
        vec![
            RankedDocument {
                doc_id: "d1".into(),
                url: "https://s1.test/".into(),
                title: "S1".into(),
                text: "Rust is a systems language. Memory safety is popular.".into(),
            },
            RankedDocument {
                doc_id: "d2".into(),
                url: "https://s2.test/".into(),
                title: "S2".into(),
                text: "Deterministic builds keep provenance intact.".into(),
            },
        ]
    }

    #[test]
    fn the_mock_refuses_an_unpinned_model() {
        let mock = MockProvider::new("m-1");
        let prompt = assemble("q", &[]);
        let result = mock.complete(&CompletionRequest {
            model: "other-model".into(),
            system: prompt.system.into(),
            user: prompt.user,
            max_tokens: 512,
        });
        assert!(result.is_err(), "unpinned model id is a caller bug");
    }

    #[test]
    fn the_mock_extractive_default_cites_back_the_source_ordinals() {
        let sources = prepare_sources(&fixture_sources(), 6000, 10);
        let prompt = assemble("What keeps Rust safe?", &sources);
        let mock = MockProvider::new("m-1");
        let resp = mock
            .complete(&CompletionRequest {
                model: "m-1".into(),
                system: prompt.system.into(),
                user: prompt.user,
                max_tokens: 512,
            })
            .unwrap();
        assert!(resp.text.contains("[1]"), "cites [1]: {}", resp.text);
        assert!(resp.text.contains("[2]"));
        assert!(resp.text.contains("Rust is a systems language"));
    }

    #[tokio::test]
    async fn the_engine_path_accepts_a_mocked_response() {
        let provider = ModelProvider::Mock(MockProvider::new("m-1"));
        let sources = prepare_sources(&fixture_sources(), 6000, 10);
        let prompt = assemble("q", &sources);
        let resp = provider
            .complete(&CompletionRequest {
                model: "m-1".into(),
                system: prompt.system.into(),
                user: prompt.user,
                max_tokens: 512,
            })
            .await;
        match resp {
            Ok(resp) => assert!(!resp.text.is_empty()),
            Err(err) => panic!("mock must not fail: {err:?}"),
        }
    }
}
