//! API error envelope, matching `docs/api/errors.md`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::error;

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
    pub message: &'static str,
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
    IndexUnavailable,
    Internal,
}

impl ApiErrorKind {
    fn into_api_error(self) -> ApiError {
        match self {
            ApiErrorKind::InvalidQuery => ApiError {
                code: "INVALID_QUERY",
                message: "The query is missing or could not be parsed.",
                status: Some(400),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::QueryTooLong => ApiError {
                code: "QUERY_TOO_LONG",
                message: "Query truncated to 512 bytes at a term boundary.",
                status: Some(400),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::InvalidParameter => ApiError {
                code: "INVALID_PARAMETER",
                message: "A parameter failed validation.",
                status: Some(400),
                retryable: Some(false),
                retry_after_ms: None,
                detail: None,
            },
            ApiErrorKind::IndexUnavailable => ApiError {
                code: "INDEX_UNAVAILABLE",
                message: "No index generation is currently available.",
                status: Some(503),
                retryable: Some(true),
                retry_after_ms: Some(30_000),
                detail: None,
            },
            ApiErrorKind::Internal => ApiError {
                code: "INTERNAL",
                message: "An internal error occurred.",
                status: Some(500),
                retryable: Some(true),
                retry_after_ms: None,
                detail: None,
            },
        }
    }
}

impl IntoResponse for ApiErrorKind {
    fn into_response(self) -> Response {
        let err = self.into_api_error();
        let status = StatusCode::from_u16(err.status.unwrap_or(500))
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        error!(code = err.code, status = status.as_u16(), "api error");
        (
            status,
            [(lynx_logging::request::HEADER, "unknown")],
            Json(ErrorEnvelope { error: err }),
        )
            .into_response()
    }
}
