//! `lynx-ai` — Astra, the optional answer layer (spec §Q, §Z step 18).
//!
//! Off by default (`ai.enabled = false` is the shipped default and production
//! refuses `ai.fail_closed: false`). The plane:
//!
//! ```text
//! query → prepare sources (sanitise, injection-screen, budget-truncate)
//!       → prompt assemble (fixed system prompt + delimited data channel)
//!       → one model call (pinned model, temp 0)        [only this touches network]
//!       → citation verify (fail closed)                [deterministic]
//!       → answer + citations, or SERP-only refusal
//! ```
//!
//! Module map: [`sources`] provenance-preserving prep, [`prompt`] the two-field
//! prompt, [`injection`]/[`sanitise`] the content defences, [`provider`] the
//! pinch-point where model I/O lives, [`verify`] the fail-closed post-processor,
//! [`controls`] kill switch / budget / breaker / cache, [`engine`] the
//! orchestrator, [`error`] failure kinds.
//!
//! Stub established in implementation Step 1; the plane landed in Step 18.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod controls;
pub mod engine;
pub mod error;
pub mod injection;
pub mod prompt;
pub mod provider;
pub mod sanitise;
pub mod sources;
pub mod verify;

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "ai";

pub use engine::{Answer, AnswerEngine, AnswerOutcome, AnswerRefusal, AnswerRequest};
pub use error::ProviderError;
pub use verify::{Citation, GroundedSummary, ValidationError, FALLBACK_MESSAGE};

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "ai");
    }
}
