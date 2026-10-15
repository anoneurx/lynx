//! `lynx-frontier` — the crawl frontier.
//!
//! Holds URLs awaiting a fetch, ordered by priority and gated by per-host politeness
//! and daily budget. The frontier is the only component that may enqueue a fetch.
//!
//! This crate owns:
//!
//! - [`priority`] — the claim-time priority sum, including the age boost that stops
//!   a high-volume site starving the long tail.
//! - [`dedup`] — the canonical-URL dedup key, built on the shared normaliser in
//!   `lynx_common::urlnorm`.
//! - [`backoff`] — classified retry decisions and escalating host bans.
//! - [`queue`] — the [`FrontierQueue`](queue::FrontierQueue) trait and an in-memory
//!   reference implementation. Postgres (`SELECT … FOR UPDATE SKIP LOCKED`) is the
//!   production store and implements the same trait.
//!
//! See `docs/crawler/frontier.md` and ADR-0008.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp // priority/jitter values are exact and deterministic by construction
    )
)]
#![forbid(unsafe_code)]

pub mod backoff;
pub mod dedup;
pub mod error;
pub mod priority;
pub mod queue;

pub use backoff::{escalating_host_ban, BackoffPolicy, Ban, BanScope, ErrorClass, RetryPlan};
pub use dedup::{dedup_key, DedupKey};
pub use error::FrontierError;
pub use priority::{PriorityInputs, PriorityWeights};
pub use queue::{
    BoxFuture, ClaimedItem, Discovery, EnqueueOutcome, FrontierQueue, InMemoryFrontier,
    NewFrontierItem,
};

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "frontier";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "frontier");
    }
}
