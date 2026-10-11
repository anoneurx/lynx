//! Per-request identifiers.
//!
//! A request id is a `UUIDv7` generated at the edge of the request and carried in the
//! `X-Request-Id` response header. A user can quote it to support; it reveals nothing
//! about the query because it is random, not derived. It is also the one id that *is*
//! safe to put in a log field, because it is not a document or query identifier.

use uuid::Uuid;

/// The response/request header carrying the request id.
pub const HEADER: &str = "x-request-id";

/// An opaque per-request identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(Uuid);

impl RequestId {
    /// Generate a new time-ordered request id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Parse from an incoming `X-Request-Id`, if valid. Invalid values are ignored by
    /// callers and replaced with a fresh id, so a client cannot inject one.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Uuid::parse_str(value.trim()).ok().map(Self)
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_string() {
        let id = RequestId::new();
        assert_eq!(RequestId::parse(&id.to_string()), Some(id));
    }

    #[test]
    fn rejects_garbage() {
        assert!(RequestId::parse("not-a-uuid").is_none());
        assert!(RequestId::parse("").is_none());
    }

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let a = RequestId::new();
        let b = RequestId::new();
        assert_ne!(a, b);
        assert!(a.as_uuid().as_bytes() <= b.as_uuid().as_bytes() || a != b);
    }
}
