//! `lynx-ranker` — transparent multi-signal ranking.
//!
//! Scores a candidate set with an explicit, explainable combination of BM25F,
//! freshness, quality, authority, and context, then applies a multiplicative spam
//! penalty. Every result carries its signal breakdown.
//!
//! Stub established in implementation Step 1. The implementation lands in Step 5.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "ranker";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "ranker");
    }
}
