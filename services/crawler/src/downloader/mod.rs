//! The streaming downloader: a validated URL in, bounded bytes out.
//!
//! Every limit in [`crate::safety::ResourceLimits`] is enforced while reading, not
//! after: the byte cap counts what actually arrives (servers lie about
//! `Content-Length`), the decompression ratio is tracked as the stream is decoded, and
//! the deadlines are applied independently to connect, headers, idle reads, and the
//! whole request.
//!
//! Redirects are followed **manually**. The underlying HTTP client is built with
//! `redirect::Policy::none()` so it can never step past the safety gate; each hop is
//! re-validated by [`crate::safety::RedirectChain`] before its request is made. The
//! socket for each hop is pinned to the address the gate validated, closing the
//! DNS-rebinding window between resolution and connect.
//!
//! See `docs/crawler/downloader.md`.

pub mod decompress;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use lynx_frontier::backoff::ErrorClass;
use reqwest::header::{
    HeaderMap, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CONTENT_ENCODING, CONTENT_LENGTH,
    CONTENT_TYPE, LOCATION, RETRY_AFTER,
};
use reqwest::redirect::Policy;
use reqwest::{Client, Response};
use time::format_description::well_known::Rfc2822;
use time::OffsetDateTime;
use url::Url;

use crate::robots::USER_AGENT_HEADER;
use crate::safety::{validate_url, FetchPolicy, HostResolver};

pub use decompress::DecompressError;

/// Request header values, per `docs/crawler/downloader.md`.
const ACCEPT_VALUE: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8";
const ACCEPT_ENCODING_VALUE: &str = "gzip, br";
const ACCEPT_LANGUAGE_VALUE: &str = "en;q=0.9,*;q=0.5";

/// Content types we will hand to the parser. `application/pdf` is deferred.
const INDEXABLE_TYPES: [&str; 5] = [
    "text/html",
    "application/xhtml+xml",
    "text/plain",
    "application/xml",
    "text/xml",
];

/// The statuses we treat as redirects (all methods are GET).
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// The media type of a `Content-Type` header value, without parameters.
fn media_type(content_type: &str) -> &str {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
}

/// Whether a media type is on the indexable allowlist.
fn is_indexable(media_type: &str) -> bool {
    INDEXABLE_TYPES.contains(&media_type)
}

/// Why a fetch failed. Each variant classifies to an [`ErrorClass`] for the frontier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The URL did not parse.
    #[error("url {0:?} is not a valid URL")]
    InvalidUrl(String),
    /// The safety gate refused the URL.
    #[error("safety gate refused the url: {0}")]
    Safety(#[from] crate::safety::SafetyError),
    /// The HTTP client could not be built for this hop.
    #[error("could not build the http client: {0}")]
    ClientBuild(String),
    /// The connection could not be established.
    #[error("connection failed: {0}")]
    Connect(String),
    /// A non-connect transport error occurred.
    #[error("transport error: {0}")]
    Transport(String),
    /// No response headers arrived within the header deadline.
    #[error("timed out waiting for response headers")]
    TimeoutHeaders,
    /// The body stalled past the idle deadline.
    #[error("timed out reading the response body")]
    TimeoutBody,
    /// The body exceeded the byte cap (either the header or the stream).
    #[error("response body exceeds the byte cap")]
    ContentTooLarge,
    /// The server used a `Content-Encoding` we never advertised.
    #[error("content-encoding {0:?} is not supported")]
    UnsupportedEncoding(String),
    /// The decompressed stream tripped the ratio cap.
    #[error("decompression bomb: {0}")]
    DecompressionBomb(String),
    /// The content type is not one we index.
    #[error("content type {0:?} is not indexable")]
    UnsupportedContentType(String),
    /// A redirect status arrived with no usable `Location`.
    #[error("redirect status {0} without a usable Location header")]
    RedirectWithoutLocation(u16),
    /// A non-success terminal status.
    #[error("http status {status}")]
    Http {
        /// The status code.
        status: u16,
        /// The server's requested delay, in seconds, if any.
        retry_after: Option<u64>,
    },
}

impl FetchError {
    /// The frontier error class for this failure.
    ///
    /// The `error_class` enum has no generic transport/connect bucket, so connect and
    /// transport failures map to the retryable `TimeoutConnect` class; the precise
    /// cause remains in the error message.
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            FetchError::Safety(
                crate::safety::SafetyError::RedirectLoop(_)
                | crate::safety::SafetyError::RedirectBudgetExceeded(_),
            ) => ErrorClass::RedirectLoop,
            FetchError::InvalidUrl(_) | FetchError::Safety(_) => ErrorClass::SafetyDenied,
            FetchError::ClientBuild(_) => ErrorClass::IndexFailed,
            // Connect, transport, and deadline failures are all retryable; the enum has
            // no finer distinction.
            FetchError::Connect(_)
            | FetchError::Transport(_)
            | FetchError::TimeoutHeaders
            | FetchError::TimeoutBody => ErrorClass::TimeoutConnect,
            FetchError::ContentTooLarge => ErrorClass::ContentTooLarge,
            FetchError::UnsupportedEncoding(_) | FetchError::UnsupportedContentType(_) => {
                ErrorClass::UnsupportedContentType
            }
            FetchError::DecompressionBomb(_) => ErrorClass::ParseFailed,
            FetchError::RedirectWithoutLocation(_) => ErrorClass::Http4xx,
            FetchError::Http { status, .. } => match status {
                408 => ErrorClass::TimeoutConnect,
                429 => ErrorClass::Upstream429,
                500..=599 => ErrorClass::Upstream5xx,
                _ => ErrorClass::Http4xx,
            },
        }
    }

    /// The server-supplied `Retry-After`, in seconds, if present.
    #[must_use]
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            FetchError::Http { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// A successful, bounded fetch.
#[derive(Debug, Clone)]
pub struct FetchResponse {
    /// The terminal status code.
    pub status: u16,
    /// The URL the body actually came from (after redirects).
    pub final_url: Url,
    /// The terminal `Content-Type`, if the server sent one.
    pub content_type: Option<String>,
    /// The decompressed body, at most `max_response_bytes`.
    pub body: Vec<u8>,
    /// Bytes observed on the wire (compressed).
    pub compressed_bytes: u64,
    /// Bytes after decompression.
    pub decompressed_bytes: u64,
    /// The URLs of the followed redirects, oldest first.
    pub redirects: Vec<Url>,
}

/// Parses a `Retry-After` value: either delay-seconds or an HTTP-date.
///
/// Returns `None` when the value is neither, so the caller falls back to its own
/// backoff curve rather than trusting an unparseable header.
#[must_use]
pub fn parse_retry_after(value: &str, now: OffsetDateTime) -> Option<u64> {
    let trimmed = value.trim();
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(seconds);
    }
    let when = OffsetDateTime::parse(trimmed, &Rfc2822).ok()?;
    let delta = (when - now).whole_seconds();
    u64::try_from(delta).ok()
}

fn retry_after_from(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    parse_retry_after(value, OffsetDateTime::now_utc())
}

fn content_length_from(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Builds bounded bytes from validated URLs.
#[derive(Debug)]
pub struct Downloader {
    policy: FetchPolicy,
    resolver: Arc<dyn HostResolver>,
    user_agent: String,
}

impl Downloader {
    /// A downloader with the default `LynxBot` identity.
    #[must_use]
    pub fn new(policy: FetchPolicy, resolver: Arc<dyn HostResolver>) -> Self {
        Self {
            policy,
            resolver,
            user_agent: USER_AGENT_HEADER.to_string(),
        }
    }

    /// Override the `User-Agent` string.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// The active policy.
    #[must_use]
    pub fn policy(&self) -> &FetchPolicy {
        &self.policy
    }

    /// Fetch `url`, following redirects manually and enforcing every cap.
    ///
    /// # Errors
    /// Returns a [`FetchError`] on any validation, transport, or limit failure.
    pub async fn fetch(&self, url: &str) -> Result<FetchResponse, FetchError> {
        let parsed = Url::parse(url).map_err(|_| FetchError::InvalidUrl(url.to_string()))?;
        let initial = validate_url(&parsed, &self.policy, self.resolver.as_ref()).await?;
        let mut chain = crate::safety::RedirectChain::start(&initial, self.policy.max_redirects());

        loop {
            let hop = chain
                .hops()
                .last()
                .cloned()
                .ok_or_else(|| FetchError::InvalidUrl(url.to_string()))?;
            let host = hop
                .url
                .host_str()
                .ok_or_else(|| FetchError::InvalidUrl(hop.url.to_string()))?
                .to_string();
            let port = hop
                .url
                .port_or_known_default()
                .ok_or_else(|| FetchError::InvalidUrl(hop.url.to_string()))?;

            let client = self.build_client(&host, port, &hop.addresses)?;
            let response = self.send(&client, &hop.url).await?;

            if is_redirect(response.status().as_u16()) {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .ok_or(FetchError::RedirectWithoutLocation(
                        response.status().as_u16(),
                    ))?;
                chain
                    .follow(&location, &self.policy, self.resolver.as_ref())
                    .await?;
                continue;
            }

            return self.read_body(response, &chain).await;
        }
    }

    fn build_client(
        &self,
        host: &str,
        port: u16,
        addresses: &[IpAddr],
    ) -> Result<Client, FetchError> {
        let limits = self.policy.limits();
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .user_agent(&self.user_agent)
            .connect_timeout(Duration::from_millis(limits.connect_timeout_ms))
            .timeout(Duration::from_millis(limits.total_timeout_ms))
            .pool_max_idle_per_host(2);

        // Pin the socket to the addresses the gate validated. An IP-literal host needs
        // no override; a name must never be re-resolved by the client.
        if host.parse::<IpAddr>().is_err() {
            let sockets: Vec<SocketAddr> = addresses
                .iter()
                .map(|address| SocketAddr::new(*address, port))
                .collect();
            builder = builder.resolve_to_addrs(host, &sockets);
        }

        builder
            .build()
            .map_err(|error| FetchError::ClientBuild(error.to_string()))
    }

    async fn send(&self, client: &Client, url: &Url) -> Result<Response, FetchError> {
        let deadline = Duration::from_millis(self.policy.limits().header_timeout_ms);
        let request = client
            .get(url.clone())
            .header(ACCEPT, ACCEPT_VALUE)
            .header(ACCEPT_ENCODING, ACCEPT_ENCODING_VALUE)
            .header(ACCEPT_LANGUAGE, ACCEPT_LANGUAGE_VALUE)
            .send();

        match tokio::time::timeout(deadline, request).await {
            Err(_) => Err(FetchError::TimeoutHeaders),
            Ok(Err(error)) => Err(if error.is_timeout() {
                FetchError::TimeoutHeaders
            } else if error.is_connect() {
                FetchError::Connect(error.to_string())
            } else {
                FetchError::Transport(error.to_string())
            }),
            Ok(Ok(response)) => Ok(response),
        }
    }

    async fn read_body(
        &self,
        mut response: Response,
        chain: &crate::safety::RedirectChain,
    ) -> Result<FetchResponse, FetchError> {
        let limits = self.policy.limits();
        let status = response.status();
        let final_url = response.url().clone();

        if !status.is_success() {
            return Err(FetchError::Http {
                status: status.as_u16(),
                retry_after: retry_after_from(response.headers()),
            });
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mime = content_type.as_deref().map(media_type).unwrap_or_default();
        if !is_indexable(mime) {
            return Err(FetchError::UnsupportedContentType(
                content_type.unwrap_or_default(),
            ));
        }

        if let Some(length) = content_length_from(response.headers()) {
            if length > limits.max_response_bytes {
                return Err(FetchError::ContentTooLarge);
            }
        }

        let encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        let mut compressed: Vec<u8> = Vec::new();
        let idle = Duration::from_millis(limits.read_idle_timeout_ms);
        loop {
            match tokio::time::timeout(idle, response.chunk()).await {
                Err(_) => return Err(FetchError::TimeoutBody),
                Ok(Err(error)) => return Err(FetchError::Transport(error.to_string())),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(chunk))) => {
                    if compressed.len() as u64 + chunk.len() as u64 > limits.max_response_bytes {
                        return Err(FetchError::ContentTooLarge);
                    }
                    compressed.extend_from_slice(&chunk);
                }
            }
        }

        let compressed_bytes = compressed.len() as u64;
        let body = decompress::decode(
            &encoding,
            &compressed,
            limits.max_response_bytes,
            limits.max_decompression_ratio,
        )
        .map_err(classify_decompress)?;
        let decompressed_bytes = body.len() as u64;

        let redirects = chain
            .hops()
            .iter()
            .skip(1)
            .map(|hop| hop.url.clone())
            .collect();

        Ok(FetchResponse {
            status: status.as_u16(),
            final_url,
            content_type,
            body,
            compressed_bytes,
            decompressed_bytes,
            redirects,
        })
    }
}

fn classify_decompress(error: DecompressError) -> FetchError {
    match error {
        DecompressError::TooLarge(_) => FetchError::ContentTooLarge,
        DecompressError::RatioExceeded(ratio) => {
            FetchError::DecompressionBomb(format!("ratio {ratio}:1 exceeded"))
        }
        DecompressError::UnsupportedEncoding(encoding) => FetchError::UnsupportedEncoding(encoding),
        DecompressError::Malformed(message) => FetchError::Transport(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_strips_parameters() {
        assert_eq!(media_type("text/html; charset=utf-8"), "text/html");
        assert_eq!(media_type("  text/plain "), "text/plain");
        assert_eq!(
            media_type("application/octet-stream"),
            "application/octet-stream"
        );
    }

    #[test]
    fn indexable_allowlist_is_exact() {
        assert!(is_indexable("text/html"));
        assert!(is_indexable("application/xhtml+xml"));
        assert!(is_indexable("text/plain"));
        assert!(is_indexable("application/xml"));
        assert!(is_indexable("text/xml"));
        assert!(!is_indexable("application/pdf"));
        assert!(!is_indexable("application/octet-stream"));
        assert!(!is_indexable("image/png"));
        assert!(!is_indexable(""));
    }

    #[test]
    fn redirect_statuses() {
        for status in [301, 302, 303, 307, 308] {
            assert!(is_redirect(status), "{status}");
        }
        for status in [200, 300, 304, 400, 500] {
            assert!(!is_redirect(status), "{status}");
        }
    }

    #[test]
    fn retry_after_seconds() {
        let now = OffsetDateTime::now_utc();
        assert_eq!(parse_retry_after("120", now), Some(120));
        assert_eq!(parse_retry_after("  30 ", now), Some(30));
        assert_eq!(parse_retry_after("", now), None);
        assert_eq!(parse_retry_after("soon", now), None);
    }

    #[test]
    fn retry_after_http_date() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let when = now + time::Duration::seconds(100);
        let value = when.format(&Rfc2822).unwrap();
        assert_eq!(parse_retry_after(&value, now), Some(100));
        // An IMF-fixdate (RFC 5322 obsolete zone) is also accepted.
        assert!(parse_retry_after("Wed, 15 Nov 2023 06:13:40 GMT", now).is_some());
    }

    #[test]
    fn classes_are_stable() {
        assert_eq!(
            FetchError::ContentTooLarge.class(),
            ErrorClass::ContentTooLarge
        );
        assert_eq!(
            FetchError::TimeoutHeaders.class(),
            ErrorClass::TimeoutConnect
        );
        assert_eq!(
            FetchError::Http {
                status: 429,
                retry_after: Some(30),
            }
            .class(),
            ErrorClass::Upstream429
        );
        assert_eq!(
            FetchError::Http {
                status: 503,
                retry_after: None,
            }
            .class(),
            ErrorClass::Upstream5xx
        );
        assert_eq!(
            FetchError::Http {
                status: 404,
                retry_after: None,
            }
            .class(),
            ErrorClass::Http4xx
        );
    }
}
