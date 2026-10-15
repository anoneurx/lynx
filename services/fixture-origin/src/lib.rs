//! `lynx-fixture-origin` — a tiny origin server for crawler tests.
//!
//! The crawler must be tested against a server that can produce the awkward cases:
//! `robots.txt` variants (longest-match, `*`/`$`, `Crawl-delay`, no-rules, oversized),
//! transient failures (403/404/429/5xx), and redirects. Pointing tests at the public
//! internet would make them flaky and impolite; this server is deterministic and
//! local-only.
//!
//! It is **not** production code. It binds to loopback and serves fixed content.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

use std::net::{Ipv4Addr, SocketAddr};

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// The default `robots.txt` served at `/robots.txt`: allow everything.
pub const DEFAULT_ROBOTS: &str = "User-agent: *\nAllow: /\n";

/// The size the `oversized` variant exceeds.
const OVERSIZED_BYTES: usize = 256 * 1024;

/// Build the router. Exposed so tests can drive it with `tower` if they prefer.
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/robots.txt", get(default_robots))
        .route("/robots/:variant", get(robots_variant))
        .route("/page/:id", get(page))
        .route("/status/:code", get(status))
}

/// Serve the router on `listener` until it errors.
///
/// # Errors
/// Returns an error if the underlying connection accept loop fails.
pub async fn serve(listener: TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router()).await
}

/// Bind a loopback port, spawn the server, and return its address and task handle.
///
/// # Errors
/// Returns an error if the loopback socket cannot be bound.
pub async fn bind_ephemeral() -> std::io::Result<(SocketAddr, JoinHandle<()>)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = serve(listener).await;
    });
    Ok((addr, handle))
}

async fn index() -> &'static str {
    "lynx-fixture-origin\n"
}

async fn default_robots() -> Response {
    text(DEFAULT_ROBOTS)
}

async fn robots_variant(Path(variant): Path<String>) -> Response {
    match variant.as_str() {
        "allow-all" => text(DEFAULT_ROBOTS),
        "disallow-all" => text("User-agent: *\nDisallow: /\n"),
        "empty-disallow" => text("User-agent: *\nDisallow:\n"),
        "longest-match" => text("User-agent: *\nDisallow: /private/\nAllow: /private/public/\n"),
        "wildcard" => text("User-agent: *\nDisallow: /*.pdf$\n"),
        "crawl-delay" => text("User-agent: *\nCrawl-delay: 10\nDisallow: /admin\n"),
        "groups" => text("User-agent: *\nDisallow: /\n\nUser-agent: LynxBot\nDisallow: /only\n"),
        "oversized" => oversized(),
        "status-403" => StatusCode::FORBIDDEN.into_response(),
        "status-404" => StatusCode::NOT_FOUND.into_response(),
        "status-429" => cooldown(),
        "status-500" => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        "redirect" => redirect(),
        _ => (StatusCode::NOT_FOUND, "unknown robots variant\n").into_response(),
    }
}

async fn page(Path(id): Path<String>) -> Response {
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let body = format!(
        "<!doctype html><html><head><title>Fixture {safe}</title></head>\
         <body><h1>{safe}</h1><a href=\"/page/next\">next</a></body></html>"
    );
    html(&body)
}

async fn status(Path(code): Path<u16>) -> Response {
    StatusCode::from_u16(code).map_or_else(
        |_| (StatusCode::BAD_REQUEST, "invalid status code\n").into_response(),
        IntoResponse::into_response,
    )
}

fn text(body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

fn html(body: &str) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

fn oversized() -> Response {
    let mut body = String::with_capacity(OVERSIZED_BYTES + 512);
    body.push_str("User-agent: *\n");
    while body.len() <= OVERSIZED_BYTES {
        body.push_str("Disallow: /bulk/segment\n");
    }
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

fn cooldown() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "120")],
        "slow down\n",
    )
        .into_response()
}

fn redirect() -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, "/robots/allow-all")],
        "",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn body_string(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn robots_txt_defaults_to_allow_all() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/robots.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_string(response).await, DEFAULT_ROBOTS);
    }

    #[tokio::test]
    async fn robots_variant_serves_the_requested_policy() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/robots/disallow-all")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(body_string(response).await, "User-agent: *\nDisallow: /\n");
    }

    #[tokio::test]
    async fn robots_failure_variants_return_their_status() {
        for (variant, expected) in [
            ("status-403", StatusCode::FORBIDDEN),
            ("status-404", StatusCode::NOT_FOUND),
            ("status-429", StatusCode::TOO_MANY_REQUESTS),
            ("status-500", StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            let response = router()
                .oneshot(
                    Request::builder()
                        .uri(format!("/robots/{variant}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "variant {variant}");
        }
    }

    #[tokio::test]
    async fn oversized_robots_exceeds_the_cap() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/robots/oversized")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(body_string(response).await.len() > OVERSIZED_BYTES);
    }

    #[tokio::test]
    async fn redirect_sets_a_location() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/robots/redirect")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()[header::LOCATION], "/robots/allow-all");
    }
}
