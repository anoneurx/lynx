//! `lynx-query` — the query processor.
//!
//! Turns a raw query string into a typed, validated [`query`](self) plan: parse the
//! grammar, normalise and analyse the terms, and choose an execution strategy. The raw
//! string is discarded after parsing; nothing downstream can log it.
//!
//! Stub established in implementation Step 1. The implementation lands in Step 2.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "query";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "query");
    }
}
