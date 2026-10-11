//! `lynx-frontier` — the crawl frontier.
//!
//! Holds URLs awaiting a fetch, ordered by priority and gated by per-host politeness
//! and daily budget. The frontier is the only component that may enqueue a fetch.
//!
//! Stub established in implementation Step 1.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "frontier";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "frontier");
    }
}
