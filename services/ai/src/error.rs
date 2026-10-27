//! Errors for the answer plane. They all map to the same user-visible outcome —
//! "no answer, SERP only", fail closed — but the *kind* decides what the
//! operator sees in the logs.

use thiserror::Error;

/// Journey-level failure kinds.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    /// Transport; the message carries only coarse codes, never content.
    #[error("model endpoint transport failure: {0}")]
    Transport(String),
    /// The response did not match the documented schema.
    #[error("model response schema mismatch: {0}")]
    Schema(String),
    /// The request named a model the pinned provider does not serve.
    #[error("model id mismatch: requested {requested}, provider pinned {pinned}")]
    ModelMismatch {
        /// Requested model id.
        requested: String,
        /// Pinned model id.
        pinned: String,
    },
    /// A 4xx refusal from the endpoint.
    #[error("model endpoint refused the request: {0}")]
    Refused(String),
}

impl ProviderError {
    /// A schema mismatch.
    #[must_use]
    pub fn schema(message: impl Into<String>) -> Self {
        Self::Schema(message.into())
    }

    /// A pinned-model mismatch.
    #[must_use]
    pub fn model_mismatch(requested: &str, pinned: &str) -> Self {
        Self::ModelMismatch {
            requested: requested.to_string(),
            pinned: pinned.to_string(),
        }
    }
}
