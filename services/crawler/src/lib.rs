//! `lynx-crawler` — the polite, safety-gated fetcher.
//!
//! Fetches URLs claimed from the frontier, applies robots, redirect, byte-cap, and
//! decompression-ratio limits, and hands bodies to the parser. It is the only
//! component that makes outbound network requests.
//!
//! Implemented so far:
//!
//! - [`robots`] — a bounded, ReDoS-safe `robots.txt` parser, the UA group selection
//!   rules, and the fetch-failure policy. Any ambiguity resolves to disallow.
//! - [`cache`] — the host-keyed [`RobotsCache`](cache::RobotsCache) contract and an
//!   in-memory implementation, including negative caching.
//! - [`safety`] — the fetch safety gate: scheme/userinfo/port checks, numeric literal
//!   decoding, DNS resolution, deny-based address classification, DNS pinning, and
//!   per-hop redirect validation. The SSRF control.
//!
//! The streaming downloader arrives in Step 11.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod cache;
pub mod robots;
pub mod safety;

pub use cache::{cached_from_status, CachedRobots, InMemoryRobotsCache, RobotsCache};
pub use robots::{
    effective_crawl_delay_seconds, parse_capped, policy_for_status, GroupSelection, RobotsDecision,
    RobotsRules, RobotsStatusPolicy, USER_AGENT_GROUP, USER_AGENT_HEADER,
};
pub use safety::{
    classify, is_publicly_routable, validate_url, validate_url_str, AddressClass, FetchPolicy,
    HostKind, HostResolver, RedirectChain, RedirectHop, ResourceLimits, SafetyError,
    StaticResolver, ValidatedUrl,
};

/// The service name, used in logs and metrics.
pub const SERVICE_NAME: &str = "crawler";

#[cfg(test)]
mod tests {
    #[test]
    fn service_name_is_stable() {
        assert_eq!(super::SERVICE_NAME, "crawler");
    }
}
