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

use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use axum::body::{Body, Bytes};
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

/// The size `/huge` streams from.
const HUGE_BYTES: usize = 1024 * 1024;

/// The size of `/declared-large`, served with an accurate but over-cap `Content-Length`.
const DECLARED_LARGE_BYTES: usize = 1024 * 1024;

/// Build the router. Exposed so tests can drive it with `tower` if they prefer.
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/robots.txt", get(default_robots))
        .route("/robots/:variant", get(robots_variant))
        .route("/page/:id", get(page))
        .route("/status/:code", get(status))
        .route("/redirect/private", get(redirect_private))
        .route("/huge", get(huge))
        .route("/declared-large", get(declared_large))
        .route("/gzip", get(gzip_body))
        .route("/brotli", get(brotli_body))
        .route("/gzip-bomb", get(gzip_bomb))
        .route("/octet", get(octet))
        .route("/slow", get(slow))
        .route("/redirect/loop-a", get(redirect_loop_a))
        .route("/redirect/loop-b", get(redirect_loop_b))
        .route("/redirect/chain/:n", get(redirect_chain))
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
    match StatusCode::from_u16(code) {
        Err(_) => (StatusCode::BAD_REQUEST, "invalid status code\n").into_response(),
        Ok(StatusCode::TOO_MANY_REQUESTS) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "120")],
            "slow down\n",
        )
            .into_response(),
        Ok(status) => status.into_response(),
    }
}

/// A body several times the stream cap, sent with no `Content-Length` (chunked) so the
/// streaming byte cap — not the header precheck — is what refuses it.
async fn huge() -> Response {
    const CHUNK: usize = 64 * 1024;
    let chunks = HUGE_BYTES / CHUNK;
    let stream = futures_util::stream::iter(
        (0..chunks).map(|_| Ok::<Bytes, std::io::Error>(Bytes::from(vec![b'a'; CHUNK]))),
    );
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Body::from_stream(stream),
    )
        .into_response()
}

/// A body larger than a test's byte cap, sent with an accurate `Content-Length`, so
/// the header precheck refuses it before a single body byte is read.
async fn declared_large() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Bytes::from(vec![b'b'; DECLARED_LARGE_BYTES]),
    )
        .into_response()
}

/// A gzip-encoded HTML body with `Content-Encoding: gzip`.
// Writing to an in-memory `Vec` cannot fail, so the result is unwrapped.
#[allow(clippy::expect_used)]
async fn gzip_body() -> Response {
    let payload = b"<html><body>compressed fixture</body></html>".repeat(8);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&payload).expect("gzip write");
    let bytes = encoder.finish().expect("gzip finish");
    compressed(bytes, "gzip")
}

/// A brotli-encoded HTML body with `Content-Encoding: br`.
// Writing to an in-memory `Vec` cannot fail, so the result is unwrapped.
#[allow(clippy::expect_used)]
async fn brotli_body() -> Response {
    let payload = b"<html><body>brotli fixture</body></html>".repeat(8);
    let mut bytes = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut bytes, 4096, 6, 22);
        writer.write_all(&payload).expect("brotli write");
    }
    compressed(bytes, "br")
}

/// A compression bomb: megabytes of zeros that compress to a few kilobytes.
// Writing to an in-memory `Vec` cannot fail, so the result is unwrapped.
#[allow(clippy::expect_used)]
async fn gzip_bomb() -> Response {
    let payload = vec![0_u8; 4 * 1024 * 1024];
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&payload).expect("gzip write");
    let bytes = encoder.finish().expect("gzip finish");
    compressed(bytes, "gzip")
}

/// A body served with a non-indexable content type.
async fn octet() -> Response {
    (
        [(header::CONTENT_TYPE, "application/octet-stream")],
        "not for indexing\n",
    )
        .into_response()
}

/// Headers immediately, then a body that stalls past any sane idle deadline.
async fn slow() -> Response {
    let stream = futures_util::stream::once(async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"late\n"))
    });
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn redirect_loop_a() -> Response {
    redirect_to("/redirect/loop-b")
}

async fn redirect_loop_b() -> Response {
    redirect_to("/redirect/loop-a")
}

/// Redirects to `/redirect/chain/{n-1}` until `n` is zero, then serves a final page.
async fn redirect_chain(Path(n): Path<u32>) -> Response {
    if n == 0 {
        return html("<!doctype html><html><body>chain end</body></html>");
    }
    redirect_to(&format!("/redirect/chain/{}", n - 1))
}

fn redirect_to(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, location.to_string())],
        "",
    )
        .into_response()
}

fn compressed(bytes: Vec<u8>, encoding: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_ENCODING, encoding),
        ],
        Bytes::from(bytes),
    )
        .into_response()
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

/// A redirect whose target is a link-local metadata address. The safety gate must
/// refuse the next hop; a client that follows redirects transparently would not.
async fn redirect_private() -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, "http://169.254.169.254/latest/meta-data/")],
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

    #[tokio::test]
    async fn private_redirect_targets_link_local() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/redirect/private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers()[header::LOCATION],
            "http://169.254.169.254/latest/meta-data/"
        );
    }
}
