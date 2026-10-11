//! `lynx-crawler` — the polite, safety-gated fetcher.
//!
//! Fetches URLs claimed from the frontier, applies robots, redirect, byte-cap, and
//! decompression-ratio limits, and hands bodies to the parser. It is the only
//! component that makes outbound network requests.
//!
//! Stub established in implementation Step 1.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "crawler";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "crawler");
    }
}
