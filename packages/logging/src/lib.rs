//! `lynx-logging` — structured logging and the privacy-preserving log policy.
//!
//! The crate's job is to make it *hard* to log something private rather than merely
//! discouraged. The two mechanisms are:
//!
//! - [`shape`] — the only supported way to describe a query. It reduces a query to a
//!   length bucket, a token count, and a set of operator classes. There is no function
//!   that returns the query text, so a handler cannot log it even by mistake.
//! - [`request`] — per-request ids that are safe to hand back to a user (a `UUIDv7`),
//!   so support can correlate a report without anyone reading a query.
//!
//! Forbidden log fields (see `packages/logging/README.md`): `query`, `q`,
//! `search_terms`, URL params carrying user content, document ids used as fields, IP
//! addresses, user-agent strings, and raw error strings that embed a URL.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![forbid(unsafe_code)]

pub mod request;
pub mod shape;

pub use request::RequestId;
pub use shape::QueryShape;

use tracing_subscriber::EnvFilter;

/// Output format for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Structured JSON. The only format in deployed environments.
    Json,
    /// Human-friendly. Developer convenience; refused in production.
    Pretty,
}

/// Logging initialisation options.
#[derive(Debug, Clone)]
pub struct Options {
    /// Filter directive, e.g. `info` or `lynx=debug`.
    pub level: String,
    /// Desired output format.
    pub format: Format,
    /// Whether pretty output is permitted in this environment.
    pub pretty_allowed: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: Format::Json,
            pretty_allowed: false,
        }
    }
}

/// A logging setup error.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    /// Pretty format was requested where it is not allowed.
    #[error("pretty log format is not permitted in this environment; use json")]
    PrettyForbidden,
    /// The subscriber could not be installed.
    #[error("failed to install tracing subscriber: {0}")]
    Install(String),
}

/// Install the global tracing subscriber.
///
/// # Errors
/// Returns [`LoggingError::PrettyForbidden`] when [`Format::Pretty`] is requested and
/// `pretty_allowed` is false, and [`LoggingError::Install`] if a subscriber is already
/// set.
pub fn init(options: &Options) -> Result<(), LoggingError> {
    let filter = EnvFilter::try_new(&options.level).unwrap_or_else(|_| EnvFilter::new("info"));

    match options.format {
        Format::Pretty if !options.pretty_allowed => Err(LoggingError::PrettyForbidden),
        Format::Pretty => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .pretty()
            .try_init()
            .map_err(|e| LoggingError::Install(e.to_string())),
        Format::Json => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .try_init()
            .map_err(|e| LoggingError::Install(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_is_refused_when_not_allowed() {
        let opts = Options {
            format: Format::Pretty,
            pretty_allowed: false,
            ..Options::default()
        };
        assert!(matches!(init(&opts), Err(LoggingError::PrettyForbidden)));
    }
}
