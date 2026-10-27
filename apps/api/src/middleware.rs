//! Cross-cutting request middleware.
//!
//! Three layers, applied to every response:
//!
//! 1. **Request id** — every response carries `X-Request-Id` (spec §L); a client-supplied
//!    id is honoured only when it parses as a UUID, so it cannot be used to inject
//!    content into logs.
//! 2. **Security headers** — the canonical set from [`lynx_security::headers`], applied
//!    centrally so no handler can forget it. The documented cacheable endpoints
//!    (`/api/v1/status`, `/api/v1/suggestions`) keep their own `Cache-Control`;
//!    everything else is `no-store`.
//! 3. **Global Privacy Control** — a `Sec-GPC: 1` request is honoured by stripping any
//!    `Set-Cookie` from the response (defence in depth: the API sets none, and a test
//!    asserts that). See [`lynx_security::privacy`] for the reasoning.
//! 4. **Rate limiting** — the rotating salted prefix-bucket limiter from
//!    [`lynx_security::ratelimit`], keyed on the daily-rotating HMAC of the client
//!    network prefix. The limiter sees no query text and stores nothing durable.
//!
//! The trace span produced by `TraceLayer` is configured in [`crate::router`] to carry
//! only the request path — never the query string — so raising the log level cannot
//! leak query text into logs or traces.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderName, HeaderValue, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lynx_common::time::{Clock, SystemClock};
use lynx_logging::request::{RequestId, HEADER};
use lynx_security::headers::security_headers;
use lynx_security::privacy::gpc_requested;
use lynx_security::ratelimit::{client_bucket, daily_salt, SlidingWindowLimiter};
use time::Duration;

use crate::errors::ApiErrorKind;
use crate::state::AppState;

/// Requests allowed per bucket per window by default (spec §L: anonymous 30/min).
pub const DEFAULT_RATE_LIMIT: u32 = 30;

/// The sliding window for the default limit, in seconds.
pub const DEFAULT_RATE_WINDOW_SECONDS: i64 = 60;

/// Parse exactly 64 hexadecimal characters into a 32-byte secret.
fn parse_hex_32(value: &str) -> Option<[u8; 32]> {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (i, chunk) in bytes.chunks_exact(2).enumerate() {
        let hi = u8::try_from((chunk[0] as char).to_digit(16)?).ok()?;
        let lo = u8::try_from((chunk[1] as char).to_digit(16)?).ok()?;
        out[i] = hi * 16 + lo;
    }
    Some(out)
}

/// Rate-limiting and header settings, sourced from the environment or safe defaults.
#[derive(Debug)]
pub struct SecuritySettings {
    /// Requests per bucket per window; `0` disables the limiter.
    pub rate_limit: u32,
    /// The sliding window, in seconds.
    pub rate_window_seconds: i64,
    /// Server secret for the daily rate-limit salt. Rotating it invalidates every
    /// bucket, which is the point.
    pub secret: [u8; 32],
    /// Emit `Strict-Transport-Security` (TLS terminated upstream only).
    pub hsts: bool,
    /// HSTS max-age, when [`SecuritySettings::hsts`] is set.
    pub hsts_max_age: u64,
    /// How many reverse proxies in front of us may set `X-Forwarded-For` for us to
    /// trust. `0` means always use the socket address.
    pub trusted_proxy_depth: u32,
}

impl SecuritySettings {
    /// Settings used by tests and by default binaries: rate limiting disabled, no HSTS.
    ///
    /// Production builds should use [`SecurityContext::from_env`].
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            rate_limit: 0,
            rate_window_seconds: DEFAULT_RATE_WINDOW_SECONDS,
            secret: [0_u8; 32],
            hsts: false,
            hsts_max_age: 0,
            trusted_proxy_depth: 0,
        }
    }

    /// Read settings from the environment, falling back to defaults.
    ///
    /// * `LYNX_RATE_LIMIT` / `LYNX_RATE_LIMIT_WINDOW_SECONDS` — the limit and window.
    /// * `LYNX_RATE_LIMIT_SALT` — shared by every replica so buckets agree behind a load
    ///   balancer; 64 hex chars are taken literally, any other string is expanded with
    ///   BLAKE3. If unset, a fresh random secret is drawn, which restarts every bucket
    ///   on every boot.
    /// * `LYNX_HSTS` (`1`/`true`) and `LYNX_HSTS_MAX_AGE`.
    /// * `LYNX_API_TRUSTED_PROXY_DEPTH` (alias `LYNX_TRUSTED_PROXY_DEPTH`).
    ///
    /// # Errors
    /// None; every known failure mode falls back to the default.
    #[must_use]
    pub fn from_env() -> Self {
        let mut s = Self {
            rate_limit: DEFAULT_RATE_LIMIT,
            rate_window_seconds: DEFAULT_RATE_WINDOW_SECONDS,
            secret: rand::random(),
            hsts: false,
            hsts_max_age: 31_536_000,
            trusted_proxy_depth: 0,
        };
        if let Ok(v) = std::env::var("LYNX_RATE_LIMIT") {
            if let Ok(n) = v.trim().parse() {
                s.rate_limit = n;
            }
        }
        if let Ok(v) = std::env::var("LYNX_RATE_LIMIT_WINDOW_SECONDS") {
            if let Ok(n) = v.trim().parse::<i64>() {
                s.rate_window_seconds = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("LYNX_RATE_LIMIT_SALT") {
            if let Some(secret) = parse_hex_32(v.trim()) {
                s.secret = secret;
            } else if !v.trim().is_empty() {
                // A non-hex secret (e.g. the dev sample) is expanded to 32 bytes with
                // BLAKE3 so any printable string works, while staying uniform.
                s.secret = *blake3::hash(v.trim().as_bytes()).as_bytes();
            }
        }
        if let Ok(v) = std::env::var("LYNX_HSTS") {
            let v = v.trim().to_ascii_lowercase();
            s.hsts = v == "1" || v == "true" || v == "yes";
        }
        if let Ok(v) = std::env::var("LYNX_HSTS_MAX_AGE") {
            if let Ok(n) = v.trim().parse() {
                s.hsts_max_age = n;
            }
        }
        if let Ok(v) = std::env::var("LYNX_API_TRUSTED_PROXY_DEPTH")
            .or_else(|_| std::env::var("LYNX_TRUSTED_PROXY_DEPTH"))
        {
            if let Ok(n) = v.trim().parse::<u32>() {
                s.trusted_proxy_depth = n;
            }
        }
        s
    }
}

/// The shared security state: static settings plus the memory-only limiter.
///
/// The limiter is not `Clone`; it lives behind an [`Arc`] in [`crate::state::AppState`].
pub struct SecurityContext {
    /// The static settings.
    pub settings: SecuritySettings,
    /// The sliding-window limiter; state is memory-only by design.
    pub limiter: SlidingWindowLimiter,
}

impl SecurityContext {
    /// A context with settings read from the environment and an empty limiter.
    #[must_use]
    pub fn from_env() -> Arc<Self> {
        Arc::new(Self {
            settings: SecuritySettings::from_env(),
            limiter: SlidingWindowLimiter::new(),
        })
    }

    /// A disabled context for tests.
    #[must_use]
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            settings: SecuritySettings::disabled(),
            limiter: SlidingWindowLimiter::new(),
        })
    }

    /// A context with an explicit limit, for rate-limit tests.
    #[must_use]
    pub fn with_limit(limit: u32) -> Arc<Self> {
        let mut settings = SecuritySettings::disabled();
        settings.rate_limit = limit;
        Arc::new(Self {
            settings,
            limiter: SlidingWindowLimiter::new(),
        })
    }
}

impl std::fmt::Debug for SecurityContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret is deliberately never formatted.
        f.debug_struct("SecurityContext").finish_non_exhaustive()
    }
}

/// Middleware 1: attach or honour `X-Request-Id` on every response.
///
/// A well-formed inbound id is echoed back so a user can correlate their own report; a
/// malformed one is replaced by a fresh `UUIDv7`, so the header cannot be used to
/// inject arbitrary text into logs.
pub async fn attach_request_id(req: Request<Body>, next: Next) -> Response {
    let id = req
        .headers()
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(RequestId::parse)
        .unwrap_or_default();
    let mut resp = next.run(req).await;
    if let Ok(value) = HeaderValue::from_str(&id.to_string()) {
        resp.headers_mut().insert(HEADER, value);
    }
    resp
}

/// Middleware 2: apply the canonical security headers to every response.
///
/// `/api/v1/status` and `/api/v1/suggestions` are the documented cacheable endpoints;
/// they keep whichever `Cache-Control`/`Pragma` the handler set and still receive every
/// other header. `Vary: Accept, Accept-Language` is inserted when the handler did not
/// set it.
pub async fn apply_security_headers(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let cacheable = path == "/api/v1/status" || path == "/api/v1/suggestions";
    let settings = &state.security.settings;
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    for (name, value) in security_headers(settings.hsts, settings.hsts_max_age) {
        if cacheable && (name == header::CACHE_CONTROL || name == "Pragma") {
            continue;
        }
        if let Ok(v) = HeaderValue::from_str(&value) {
            headers.insert(name, v);
        }
    }
    if !headers.contains_key(header::VARY) {
        headers.insert(
            header::VARY,
            HeaderValue::from_static("Accept, Accept-Language"),
        );
    }
    resp
}

/// Middleware 3: honour Global Privacy Control (§Z Step 17).
///
/// A `Sec-GPC: 1` request gets every `Set-Cookie` removed from its response. LYNX
/// never sets cookies (spec §N: no identifiers of any kind), so on the happy path
/// this middleware is a no-op whose *effect* is only visible in the integration
/// tests that assert cookie-free responses for normal clients too. The strip is
/// still real defence in depth: a future handler that regresses into setting a
/// cookie is caught for the very clients who asked not to be tracked, before the
/// CI test can even fire.
///
/// The signal itself is deliberately not logged, counted, or stored per-visitor —
/// the [`lynx_security::privacy`] module documents why.
pub async fn privacy_gpc(req: Request<Body>, next: Next) -> Response {
    let gpc = gpc_requested(
        req.headers()
            .get_all("sec-gpc")
            .iter()
            .map(|v| v.to_str().ok()),
    );
    let mut resp = next.run(req).await;
    if gpc && resp.headers().contains_key(header::SET_COOKIE) {
        resp.headers_mut().remove(header::SET_COOKIE);
    }
    resp
}

/// Middleware 4: rate limit `/api/v1/search` with the rotating salted prefix bucket.
///
/// `/api/v1/status` is exempt: it is cached at the edge for a minute by contract, and
/// health-check loops must not starve the search surface's budget. When the limit is
/// `0` the middleware passes through unchanged.
pub async fn rate_limit(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let settings = &state.security.settings;
    let path = req.uri().path();
    if path != "/api/v1/search" || settings.rate_limit == 0 {
        return next.run(req).await;
    }
    let now = state
        .clock
        .clone()
        .map_or_else(|| SystemClock.now(), |c| c.now());
    let salt = daily_salt(&settings.secret, now);
    let depth = settings.trusted_proxy_depth;
    let bucket = client_bucket(client_address(&req, depth), &salt);
    let window = Duration::seconds(settings.rate_window_seconds.max(1));
    let decision = state
        .security
        .limiter
        .check(&bucket, settings.rate_limit, window, now);
    if !decision.allowed {
        let retry_after_ms: u64 = decision
            .retry_after
            .map_or_else(|| ms_clamped(window), ms_clamped);
        let mut resp = ApiErrorKind::RateLimited(crate::errors::RateLimitContext {
            limit: u64::from(settings.rate_limit),
            window_seconds: u64::try_from(window.whole_seconds().max(1)).unwrap_or(u64::MAX),
            retry_after_ms,
        })
        .into_response();
        set_headers(
            &mut resp,
            &[
                ("retry-after", (retry_after_ms / 1000).max(1).to_string()),
                ("x-ratelimit-limit", settings.rate_limit.to_string()),
                ("x-ratelimit-remaining", "0".to_string()),
                (
                    "x-ratelimit-reset",
                    (retry_after_ms / 1000).max(1).to_string(),
                ),
            ],
        );
        return resp;
    }
    let mut resp = next.run(req).await;
    let remaining_reset = decision.retry_after.map_or("0".to_string(), |d| {
        u64::try_from(d.whole_seconds().max(0))
            .unwrap_or(u64::MAX)
            .to_string()
    });
    set_headers(
        &mut resp,
        &[
            ("x-ratelimit-limit", settings.rate_limit.to_string()),
            ("x-ratelimit-remaining", decision.remaining.to_string()),
            ("x-ratelimit-reset", remaining_reset),
        ],
    );
    resp
}

/// Insert or replace each `(name, value)` on a response.
fn set_headers(resp: &mut Response, pairs: &[(&'static str, String)]) {
    for (name, value) in pairs {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            if let Ok(v) = HeaderValue::from_str(value) {
                resp.headers_mut().insert(name, v);
            }
        }
    }
}

/// A duration in milliseconds, clamped to `u64`'s range.
#[must_use]
fn ms_clamped(duration: Duration) -> u64 {
    u64::try_from(duration.whole_milliseconds().max(0)).unwrap_or(u64::MAX)
}

/// The client address for bucketing, honouring the trusted-proxy depth.
///
/// With depth `d`, the address `d` hops back from the right of `X-Forwarded-For` is
/// trusted; anything shallower is attacker-controlled and ignored. Without the header
/// (or with depth `0`), the socket address is used. When no socket address is recorded
/// (direct router calls from tests), the loopback address is used so requests share a
/// single, isolated bucket.
fn client_address(req: &Request<Body>, depth: u32) -> IpAddr {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::LOCALHOST), |info| info.0.ip());
    if depth == 0 {
        return peer;
    }
    let header = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    let Some(header) = header else {
        return peer;
    };
    let addrs: Vec<IpAddr> = header
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    if addrs.len() > depth as usize {
        addrs[addrs.len() - 1 - depth as usize]
    } else {
        peer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_secrets() {
        let hex = "ab".repeat(32);
        assert!(parse_hex_32(&hex).is_some());
        assert!(parse_hex_32("abcd").is_none()); // wrong length
        assert!(parse_hex_32(&"zz".repeat(32)).is_none()); // not hex
    }

    #[test]
    fn disabled_settings_never_limit() {
        let s = SecuritySettings::disabled();
        assert_eq!(s.rate_limit, 0);
    }

    #[test]
    fn from_env_defaults() {
        let s = SecuritySettings::from_env();
        assert_eq!(s.rate_limit, DEFAULT_RATE_LIMIT);
        assert!(!s.hsts);
        assert!(s.secret != [0_u8; 32], "the default secret is random");
    }
}
