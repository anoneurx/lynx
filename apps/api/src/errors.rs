//! API error envelope, matching `docs/api/errors.md`.

use axum::{
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::error;

/// Context carried by the `RATE_LIMITED` error.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RateLimitContext {
    /// The request limit within the window.
    pub limit: u64,
    /// The window in seconds.
    pub window_seconds: u64,
    /// When to retry, in milliseconds.
    pub retry_after_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: &'static str,
    /// A human-oriented message. Rewordable; clients must not parse it. `Cow` allows
    /// computed messages (e.g. cooldowns) without materialising a leak.
    pub message: std::borrow::Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<ErrorDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub error: ApiError,
}

#[derive(Debug)]
pub enum ApiErrorKind {
    InvalidQuery,
    QueryTooLong,
    InvalidParameter,
    /// `429`: the caller's bucket is exhausted. Carries the cooldown context.
    RateLimited(RateLimitContext),
    NotFound,
    IndexUnavailable,
    Internal,
}

/// An error plus the extra response headers a kind requires (`Retry-After`,
/// `X-RateLimit-*`). Built by [`ApiErrorKind::into_response`].
struct ErrorResponse {
    envelope: ErrorEnvelope,
    status: StatusCode,
    headers: Vec<(HeaderName, String)>,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        let mut resp = (
            self.status,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            Json(self.envelope),
        )
            .into_response();
        for (name, value) in self.headers {
            if let Ok(v) = HeaderValue::from_str(&value) {
                resp.headers_mut().insert(name, v);
            }
        }
        resp
    }
}

impl ApiErrorKind {
    fn into_api_error(self) -> ApiError {
        match self {
            ApiErrorKind::InvalidQuery => ApiError {
                code: "INVALID_QUERY",
                message: "The query is missing or could not be parsed.".into(),
                status: Some(400),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::QueryTooLong => ApiError {
                code: "QUERY_TOO_LONG",
                message: "Query truncated to 512 bytes at a term boundary.".into(),
                status: Some(400),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::InvalidParameter => ApiError {
                code: "INVALID_PARAMETER",
                message: "A parameter failed validation.".into(),
                status: Some(400),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::RateLimited(ctx) => {
                let secs = (ctx.retry_after_ms / 1000).max(1);
                ApiError {
                    code: "RATE_LIMITED",
                    message: format!("Too many requests. Retry after {secs} seconds.").into(),
                    status: Some(429),
                    retryable: Some(true),
                    retry_after_ms: Some(ctx.retry_after_ms),
                    detail: Some(ErrorDetail {
                        parameter: None,
                        value: None,
                        original_length: None,
                        used_length: None,
                        limit: Some(ctx.limit),
                        window_seconds: Some(ctx.window_seconds),
                    }),
                }
            }
            ApiErrorKind::NotFound => ApiError {
                code: "NOT_FOUND",
                message: "No such resource. Note: an empty result set is a 200, not a 404.".into(),
                status: Some(404),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::IndexUnavailable => ApiError {
                code: "INDEX_UNAVAILABLE",
                message: "No index generation is currently available.".into(),
                status: Some(503),
                retryable: Some(true),
                retry_after_ms: Some(30_000),
                detail: None,
            },
            ApiErrorKind::Internal => ApiError {
                code: "INTERNAL",
                message: "An internal error occurred.".into(),
                status: Some(500),
                retryable: Some(true),
                retry_after_ms: None,
                detail: None,
            },
        }
    }

    /// The response headers this kind requires, beyond the envelope body.
    fn required_headers(err: &ApiError) -> Vec<(HeaderName, String)> {
        let mut headers = Vec::new();
        if let Some(ms) = err.retry_after_ms {
            // `Retry-After` is in whole seconds (errors.md HTTP semantics).
            headers.push((header::RETRY_AFTER, (ms / 1000).max(1).to_string()));
        }
        headers
    }
}

impl IntoResponse for ApiErrorKind {
    fn into_response(self) -> Response {
        let err = self.into_api_error();
        let status = StatusCode::from_u16(err.status.unwrap_or(500))
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        error!(code = err.code, status = status.as_u16(), "api error");
        // The request-id header is written by the middleware; the placeholder here keeps
        // a response valid even when the middleware is somehow bypassed.
        let mut headers: Vec<(HeaderName, String)> = Self::required_headers(&err);
        headers.push((
            HeaderName::from_static("x-request-id"),
            "unknown".to_string(),
        ));
        ErrorResponse {
            envelope: ErrorEnvelope { error: err },
            status,
            headers,
        }
        .into_response()
    }
}
