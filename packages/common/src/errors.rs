//! Error taxonomy: one enum of stable, machine-readable codes and their HTTP mapping.
//!
//! `code` is the contract; messages are free to change. This is the single source of
//! truth for `docs/api/errors.md`. Never place query text in an error message, detail
//! map, or request id.

use std::fmt;

/// Stable, machine-readable error codes. Meanings never change within a major version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // variant names are the documentation; see docs/api/errors.md
pub enum ErrorCode {
    InvalidQuery,
    QueryTooLong,
    InvalidParameter,
    InvalidOperator,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    RateLimited,
    QueryComplexityExceeded,
    IndexUnavailable,
    /// Partial results; the request succeeded with a caveat.
    ShardTimeout,
    /// Served uncached; the response is correct.
    CacheUnavailable,
    Internal,
}

impl ErrorCode {
    /// The stable wire representation, e.g. `"RATE_LIMITED"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidQuery => "INVALID_QUERY",
            Self::QueryTooLong => "QUERY_TOO_LONG",
            Self::InvalidParameter => "INVALID_PARAMETER",
            Self::InvalidOperator => "INVALID_OPERATOR",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::MethodNotAllowed => "METHOD_NOT_ALLOWED",
            Self::RateLimited => "RATE_LIMITED",
            Self::QueryComplexityExceeded => "QUERY_COMPLEXITY_EXCEEDED",
            Self::IndexUnavailable => "INDEX_UNAVAILABLE",
            Self::ShardTimeout => "SHARD_TIMEOUT",
            Self::CacheUnavailable => "CACHE_UNAVAILABLE",
            Self::Internal => "INTERNAL",
        }
    }

    /// The HTTP status this code maps to.
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::InvalidQuery
            | Self::QueryTooLong
            | Self::InvalidParameter
            | Self::InvalidOperator
            | Self::QueryComplexityExceeded => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::RateLimited => 429,
            Self::Internal => 500,
            Self::IndexUnavailable => 503,
            // Degraded-but-successful searches: the request worked, with a caveat.
            Self::ShardTimeout | Self::CacheUnavailable => 200,
        }
    }

    /// Whether a client can reasonably retry the request unchanged.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::IndexUnavailable | Self::Internal
        )
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A LYNX error: a stable [`ErrorCode`] plus a human message and optional detail.
///
/// The message is for a human and must never contain query text. The `detail` map is
/// endpoint-specific context and is likewise query-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LynxError {
    /// Machine-readable code.
    pub code: ErrorCode,
    /// Human-readable explanation. May be reworded; never parsed by clients.
    pub message: String,
    /// Optional retry hint in milliseconds. Present when [`ErrorCode::retryable`].
    pub retry_after_ms: Option<u64>,
    /// Endpoint-specific context. Keys may be added; never required. Query-free.
    pub detail: Vec<(String, String)>,
}

impl LynxError {
    /// Construct an error from a code and message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after_ms: None,
            detail: Vec::new(),
        }
    }

    /// Attach a retry hint.
    #[must_use]
    pub fn with_retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }

    /// Attach a key/value detail entry.
    #[must_use]
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.detail.push((key.into(), value.into()));
        self
    }

    /// The HTTP status for this error.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.code.status()
    }
}

impl fmt::Display for LynxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LynxError {}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, LynxError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_map_to_documented_statuses() {
        assert_eq!(ErrorCode::InvalidQuery.status(), 400);
        assert_eq!(ErrorCode::RateLimited.status(), 429);
        assert_eq!(ErrorCode::IndexUnavailable.status(), 503);
        // Degraded-but-successful searches stay 200.
        assert_eq!(ErrorCode::ShardTimeout.status(), 200);
        assert_eq!(ErrorCode::CacheUnavailable.status(), 200);
    }

    #[test]
    fn only_transient_faults_are_retryable() {
        assert!(ErrorCode::RateLimited.retryable());
        assert!(ErrorCode::IndexUnavailable.retryable());
        assert!(!ErrorCode::InvalidQuery.retryable());
        assert!(!ErrorCode::Forbidden.retryable());
    }

    #[test]
    fn display_uses_stable_code() {
        let err = LynxError::new(ErrorCode::InvalidParameter, "count must be 1..=100")
            .with_detail("parameter", "count")
            .with_retry_after_ms(1000);
        assert_eq!(err.to_string(), "INVALID_PARAMETER: count must be 1..=100");
        assert_eq!(err.status(), 400);
        assert_eq!(err.detail.len(), 1);
    }

    #[test]
    fn wire_codes_are_uppercase_snake() {
        for code in [
            ErrorCode::InvalidQuery,
            ErrorCode::QueryComplexityExceeded,
            ErrorCode::CacheUnavailable,
        ] {
            let s = code.as_str();
            assert!(s.chars().all(|c| c.is_ascii_uppercase() || c == '_'));
        }
    }
}
