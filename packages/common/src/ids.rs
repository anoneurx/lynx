//! Typed identifiers based on `UUIDv7`.
//!
//! v7 is time-ordered, giving B-tree locality on insert and update, and is not
//! enumerable into a scan of neighbours the way a sequential integer is. See
//! ADR-0017.
//!
//! ```
//! use lynx_common::ids::DocumentId;
//! let a = DocumentId::new();
//! let b = DocumentId::new();
//! assert!(a.to_string().parse::<DocumentId>().is_ok());
//! ```

use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", serde(transparent))]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a new, time-ordered identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wrap an existing UUID.
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The underlying UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }

            /// The 16 raw bytes, big-endian.
            #[must_use]
            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

id_type!(
    /// Stable identity for a canonical URL.
    UrlId
);
id_type!(
    /// A registrable domain or host row.
    HostId
);
id_type!(
    /// A crawl job / frontier item.
    JobId
);
id_type!(
    /// A domain governance row.
    DomainId
);

/// Identity of a document (the current indexed state of a URL).
///
/// Derived from the canonical URL so a re-crawl updates the same document in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct DocumentId(Uuid);

impl DocumentId {
    /// Derive a document id deterministically from a canonical URL.
    ///
    /// Uses a UUIDv5-style construction over a frozen namespace so the same URL always
    /// maps to the same id, across processes and restarts.
    #[must_use]
    pub fn from_canonical_url(canonical_url: &str) -> Self {
        // Fixed namespace for LYNX document ids. Never change once shipped.
        const NAMESPACE: Uuid = Uuid::from_bytes([
            0x6c, 0x79, 0x6e, 0x78, 0x2d, 0x64, 0x6f, 0x63, 0x2d, 0x76, 0x31, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]);
        Self(Uuid::new_v5(&NAMESPACE, canonical_url.as_bytes()))
    }

    /// Generate a fresh, time-ordered document id (for documents without a URL).
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wrap an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for DocumentId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_id_is_deterministic_from_url() {
        let a = DocumentId::from_canonical_url("https://example.com/page");
        let b = DocumentId::from_canonical_url("https://example.com/page");
        let c = DocumentId::from_canonical_url("https://example.com/other");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn ids_round_trip_through_strings() {
        let id = UrlId::new();
        let parsed: UrlId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn generated_ids_are_time_ordered() {
        let a = JobId::new();
        let b = JobId::new();
        // v7 ordering: later id sorts at or after earlier id.
        assert!(a <= b, "expected {a} <= {b}");
    }

    #[test]
    fn invalid_uuids_are_rejected() {
        assert!("not-a-uuid".parse::<DocumentId>().is_err());
    }
}
