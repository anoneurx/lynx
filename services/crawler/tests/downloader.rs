//! End-to-end downloader behaviour against the fixture origin (loopback only).
//!
//! Every cap the downloader claims to enforce is exercised here with a server that
//! tries to violate it: a streamed body over the limit, a declared length over the
//! cap, a gzip bomb, a stalled body, a redirect loop, a redirect into link-local
//! space, a non-web scheme, and a redirect chain past the budget.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::IpAddr;
use std::sync::Arc;

use lynx_crawler::downloader::{Downloader, FetchError};
use lynx_crawler::safety::{FetchPolicy, ResourceLimits, StaticResolver};
use lynx_fixture_origin::bind_ephemeral;
use lynx_frontier::backoff::ErrorClass;

fn limits() -> ResourceLimits {
    ResourceLimits {
        max_response_bytes: 64 * 1024,
        max_decompression_ratio: 100,
        connect_timeout_ms: 1_000,
        header_timeout_ms: 1_000,
        read_idle_timeout_ms: 1_000,
        total_timeout_ms: 5_000,
    }
}

fn policy(port: u16, limits: ResourceLimits) -> FetchPolicy {
    FetchPolicy::new(vec![port], 5, limits)
        .unwrap()
        .dangerously_allow_private_addresses()
}

fn downloader(port: u16, limits: ResourceLimits) -> Downloader {
    Downloader::new(policy(port, limits), Arc::new(StaticResolver::new()))
}

async fn server() -> (u16, String) {
    let (addr, handle) = bind_ephemeral().await.unwrap();
    drop(handle);
    (addr.port(), format!("http://{addr}"))
}

#[tokio::test]
async fn serves_a_page() {
    let (port, base) = server().await;
    let response = downloader(port, limits())
        .fetch(&format!("{base}/page/1"))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        response.content_type.as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert!(String::from_utf8_lossy(&response.body).contains("Fixture 1"));
    assert!(response.redirects.is_empty());
    assert_eq!(response.compressed_bytes, response.decompressed_bytes);
}

#[tokio::test]
async fn follows_a_redirect_chain() {
    let (port, base) = server().await;
    let response = downloader(port, limits())
        .fetch(&format!("{base}/redirect/chain/3"))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.redirects.len(), 3);
    assert!(response.final_url.path().ends_with("/redirect/chain/0"));
    assert!(String::from_utf8_lossy(&response.body).contains("chain end"));
}

#[tokio::test]
async fn refuses_a_redirect_into_private_space() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/redirect/private"))
        .await
        .unwrap_err();
    assert!(matches!(error, FetchError::Safety(_)), "{error:?}");
    assert_eq!(error.class(), ErrorClass::SafetyDenied);
}

#[tokio::test]
async fn detects_a_redirect_loop() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/redirect/loop-a"))
        .await
        .unwrap_err();
    assert_eq!(error.class(), ErrorClass::RedirectLoop);
}

#[tokio::test]
async fn refuses_a_streamed_body_over_the_cap() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/huge"))
        .await
        .unwrap_err();
    assert_eq!(error, FetchError::ContentTooLarge);
}

#[tokio::test]
async fn refuses_a_declared_length_over_the_cap() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/declared-large"))
        .await
        .unwrap_err();
    assert_eq!(error, FetchError::ContentTooLarge);
}

#[tokio::test]
async fn rejects_non_web_schemes() {
    let (port, _base) = server().await;
    for url in [
        "file:///etc/passwd",
        "gopher://example.com/1selector",
        "javascript:alert(1)",
        "data:text/html,hello",
    ] {
        let error = downloader(port, limits()).fetch(url).await.unwrap_err();
        assert_eq!(error.class(), ErrorClass::SafetyDenied, "{url}");
        assert!(matches!(error, FetchError::Safety(_)), "{url}: {error:?}");
    }
}

#[tokio::test]
async fn exhausts_the_redirect_budget() {
    // A chain longer than the policy allows must be cut, not followed.
    let (port, base) = server().await;
    let strict = FetchPolicy::new(vec![port], 2, limits())
        .unwrap()
        .dangerously_allow_private_addresses();
    let error = Downloader::new(strict, Arc::new(StaticResolver::new()))
        .fetch(&format!("{base}/redirect/chain/3"))
        .await
        .unwrap_err();
    assert_eq!(error.class(), ErrorClass::RedirectLoop, "{error:?}");
}

#[tokio::test]
async fn the_global_budget_bounds_even_a_patient_idle_timeout() {
    // `read_idle_timeout_ms` alone would wait 5 s; the whole fetch must give up much
    // sooner because the *total* budget (300 ms) is shared across every deadline.
    let (port, base) = server().await;
    let mut tight = limits();
    tight.read_idle_timeout_ms = 5_000;
    tight.total_timeout_ms = 300;
    let started = std::time::Instant::now();
    let error = downloader(port, tight)
        .fetch(&format!("{base}/slow"))
        .await
        .unwrap_err();
    // Either the body idle or the request timeout fires; both are retryable connect-
    // class. What matters is the 5 s idle sacrifice to the 300 ms budget.
    assert_eq!(error.class(), ErrorClass::TimeoutConnect, "{error:?}");
    assert!(started.elapsed() < std::time::Duration::from_millis(2_000));
}

#[tokio::test]
async fn follows_a_long_chain_within_the_global_budget() {
    // Nine quick hops under a single 5 s budget: per-hop caps are clamped by whatever
    // is left, so a fast chain is never penalised.
    let (port, base) = server().await;
    let wide = FetchPolicy::new(vec![port], 9, limits())
        .unwrap()
        .dangerously_allow_private_addresses();
    let response = Downloader::new(wide, Arc::new(StaticResolver::new()))
        .fetch(&format!("{base}/redirect/chain/9"))
        .await
        .unwrap();
    assert_eq!(response.redirects.len(), 9);
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn decompresses_gzip() {
    let (port, base) = server().await;
    let response = downloader(port, limits())
        .fetch(&format!("{base}/gzip"))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("compressed fixture"));
    assert!(response.compressed_bytes < response.decompressed_bytes);
}

#[tokio::test]
async fn decompresses_brotli() {
    let (port, base) = server().await;
    let response = downloader(port, limits())
        .fetch(&format!("{base}/brotli"))
        .await
        .unwrap();
    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("brotli fixture"));
    assert!(response.compressed_bytes < response.decompressed_bytes);
}

#[tokio::test]
async fn aborts_a_decompression_bomb() {
    let (port, base) = server().await;
    let mut bomb = limits();
    bomb.max_response_bytes = 16 * 1024 * 1024;
    bomb.max_decompression_ratio = 10;
    bomb.header_timeout_ms = 5_000;
    let error = downloader(port, bomb)
        .fetch(&format!("{base}/gzip-bomb"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, FetchError::DecompressionBomb(_)),
        "{error:?}"
    );
    assert_eq!(error.class(), ErrorClass::ParseFailed);
}

#[tokio::test]
async fn refuses_a_non_indexable_content_type() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/octet"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, FetchError::UnsupportedContentType(_)),
        "{error:?}"
    );
    assert_eq!(error.class(), ErrorClass::UnsupportedContentType);
}

#[tokio::test]
async fn times_out_on_a_stalled_body() {
    let (port, base) = server().await;
    let mut idle = limits();
    idle.read_idle_timeout_ms = 200;
    idle.total_timeout_ms = 5_000;
    let error = downloader(port, idle)
        .fetch(&format!("{base}/slow"))
        .await
        .unwrap_err();
    assert_eq!(error, FetchError::TimeoutBody);
}

#[tokio::test]
async fn status_429_carries_retry_after() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/status/429"))
        .await
        .unwrap_err();
    assert_eq!(error.retry_after(), Some(120));
    assert_eq!(error.class(), ErrorClass::Upstream429);
}

#[tokio::test]
async fn status_503_is_retryable() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/status/503"))
        .await
        .unwrap_err();
    assert_eq!(error.class(), ErrorClass::Upstream5xx);
    assert_eq!(error.retry_after(), None);
}

#[tokio::test]
async fn status_404_is_permanent() {
    let (port, base) = server().await;
    let error = downloader(port, limits())
        .fetch(&format!("{base}/status/404"))
        .await
        .unwrap_err();
    assert_eq!(error.class(), ErrorClass::Http4xx);
}

#[tokio::test]
async fn pins_the_resolved_address_for_a_hostname() {
    let (port, _base) = server().await;
    let resolver = StaticResolver::new().with("fixture.test", [IpAddr::from([127, 0, 0, 1])]);
    let fetch = Downloader::new(policy(port, limits()), Arc::new(resolver));
    // The URL names a host that only the resolver (and the pin) can turn into loopback.
    let response = fetch
        .fetch(&format!("http://fixture.test:{port}/page/pinned"))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert!(String::from_utf8_lossy(&response.body).contains("pinned"));
}

#[tokio::test]
async fn rejects_a_non_http_scheme() {
    let (port, _base) = server().await;
    let error = downloader(port, limits())
        .fetch("ftp://example.com/file")
        .await
        .unwrap_err();
    assert_eq!(error.class(), ErrorClass::SafetyDenied);
}
