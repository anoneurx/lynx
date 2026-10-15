//! Frontier errors.

/// Anything the frontier can refuse to do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FrontierError {
    /// The URL is not a valid absolute `http`/`https` URL and cannot be queued.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// No entry with the given id exists.
    #[error("no frontier entry with id {0}")]
    NotFound(uuid::Uuid),
    /// The in-memory queue lock was poisoned by a panicking thread.
    #[error("frontier queue lock poisoned")]
    Poisoned,
}
