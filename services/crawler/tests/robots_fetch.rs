//! End-to-end `robots.txt` handling: fetch from the fixture origin, parse the body,
//! and assert the resulting policy.
//!
//! These use the real network stack (loopback only) so the fetch → parse → decide
//! path is exercised as the downloader will exercise it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lynx_crawler::{
    parse_capped, policy_for_status, RobotsDecision, RobotsRules, RobotsStatusPolicy,
    USER_AGENT_GROUP,
};
use lynx_fixture_origin::bind_ephemeral;

async fn base_url() -> String {
    let (addr, handle) = bind_ephemeral().await.unwrap();
    // Detach the server task; it is cancelled when the test runtime shuts down.
    drop(handle);
    format!("http://{addr}")
}

async fn fetch(path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{}{path}", base_url().await))
        .send()
        .await
        .unwrap()
}

fn parse_from_body(body: &[u8]) -> (RobotsRules, bool) {
    parse_capped(body)
}

#[tokio::test]
async fn default_robots_allows_everything() {
    let response = fetch("/robots.txt").await;
    assert_eq!(response.status().as_u16(), 200);
    let (rules, truncated) = parse_from_body(&response.bytes().await.unwrap());
    assert!(!truncated);
    let selection = rules.select(USER_AGENT_GROUP);
    assert_eq!(selection.decision("/anything"), RobotsDecision::Allow);
}

#[tokio::test]
async fn disallow_all_is_honoured() {
    let response = fetch("/robots/disallow-all").await;
    let (rules, _) = parse_from_body(&response.bytes().await.unwrap());
    let selection = rules.select(USER_AGENT_GROUP);
    assert_eq!(selection.decision("/anything"), RobotsDecision::Disallow);
}

#[tokio::test]
async fn longest_match_and_allow_override_survive_the_round_trip() {
    let response = fetch("/robots/longest-match").await;
    let (rules, _) = parse_from_body(&response.bytes().await.unwrap());
    let selection = rules.select(USER_AGENT_GROUP);
    assert_eq!(
        selection.decision("/private/secret"),
        RobotsDecision::Disallow
    );
    assert_eq!(
        selection.decision("/private/public/page"),
        RobotsDecision::Allow
    );
}

#[tokio::test]
async fn wildcard_and_end_anchor_survive_the_round_trip() {
    let response = fetch("/robots/wildcard").await;
    let (rules, _) = parse_from_body(&response.bytes().await.unwrap());
    let selection = rules.select(USER_AGENT_GROUP);
    assert_eq!(selection.decision("/doc.pdf"), RobotsDecision::Disallow);
    assert_eq!(selection.decision("/doc.pdf.html"), RobotsDecision::Allow);
}

#[tokio::test]
async fn crawl_delay_is_read() {
    let response = fetch("/robots/crawl-delay").await;
    let (rules, _) = parse_from_body(&response.bytes().await.unwrap());
    assert_eq!(
        rules.select(USER_AGENT_GROUP).crawl_delay_seconds(),
        Some(10)
    );
}

#[tokio::test]
async fn explicit_lynx_group_beats_wildcard() {
    let response = fetch("/robots/groups").await;
    let (rules, _) = parse_from_body(&response.bytes().await.unwrap());
    let selection = rules.select(USER_AGENT_GROUP);
    assert_eq!(selection.decision("/anything"), RobotsDecision::Allow);
    assert_eq!(selection.decision("/only"), RobotsDecision::Disallow);
}

#[tokio::test]
async fn oversized_body_is_truncated_not_rejected() {
    let response = fetch("/robots/oversized").await;
    assert_eq!(response.status().as_u16(), 200);
    let body = response.bytes().await.unwrap();
    assert!(body.len() > 256 * 1024);
    let (_rules, truncated) = parse_from_body(&body);
    assert!(truncated);
}

#[tokio::test]
async fn failure_statuses_map_to_conservative_policies() {
    let cases = [
        ("status-403", RobotsStatusPolicy::DisallowAll),
        ("status-404", RobotsStatusPolicy::AllowAll),
        ("status-429", RobotsStatusPolicy::Cooldown),
        ("status-500", RobotsStatusPolicy::DisallowAllRetry),
    ];
    for (variant, expected) in cases {
        let response = fetch(&format!("/robots/{variant}")).await;
        let status = response.status().as_u16();
        assert_eq!(policy_for_status(status), expected, "variant {variant}");
    }
}

#[tokio::test]
async fn redirect_is_followed_to_the_allowed_policy() {
    let response = fetch("/robots/redirect").await;
    // reqwest follows the 302 by default; final document is the allow-all robots.
    assert_eq!(response.status().as_u16(), 200);
    let (rules, _) = parse_from_body(&response.bytes().await.unwrap());
    assert_eq!(
        rules.select(USER_AGENT_GROUP).decision("/x"),
        RobotsDecision::Allow
    );
}
