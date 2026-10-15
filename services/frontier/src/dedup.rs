//! The frontier dedup key.
//!
//! The key is the SHA-256 of the *canonical* URL — the same normalisation the API
//! and indexer use (`lynx_common::urlnorm`). Dedup is a queue property, not a data
//! loss: two distinct URLs that canonicalise identically are one queue entry, but
//! both remain addressable.
//!
//! Sorting and de-duplicating query parameters is security-adjacent, not tidiness:
//! a host that serves `?p=1..999999` in two parameter orders is two million queue
//! entries, and would otherwise consume an entire host budget on its own.

use sha2::{Digest, Sha256};

use crate::error::FrontierError;

/// A 32-byte canonical-URL digest. Comparable, hashable, and printed as hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DedupKey([u8; 32]);

impl DedupKey {
    /// The raw digest, suitable for a `bytea` column.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Debug for DedupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DedupKey({})", self.to_hex())
    }
}

impl std::fmt::Display for DedupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Canonicalise `url` and hash it to a [`DedupKey`].
///
/// # Errors
/// Returns [`FrontierError::InvalidUrl`] if the URL is not a valid absolute `http`/`https`
/// URL.
pub fn dedup_key(url: &str) -> Result<DedupKey, FrontierError> {
    let canonical = lynx_common::urlnorm::normalise(url)
        .map_err(|e| FrontierError::InvalidUrl(e.to_string()))?;
    Ok(of_canonical(&canonical))
}

/// Hash an already-canonical URL. Callers that hold a canonical string avoid the
/// parse cost; the result is identical to [`dedup_key`].
#[must_use]
pub fn of_canonical(canonical: &str) -> DedupKey {
    let digest = Sha256::digest(canonical.as_bytes());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    DedupKey(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn tracking_params_and_order_collapse() {
        let a = dedup_key("https://e.com/p?b=2&a=1&utm_source=x").unwrap();
        let b = dedup_key("https://e.com/p?a=1&b=2").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn case_and_fragment_collapse() {
        let a = dedup_key("HTTP://Example.COM/Path#frag").unwrap();
        let b = dedup_key("http://example.com/Path").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_paths_differ() {
        assert_ne!(
            dedup_key("https://e.com/a").unwrap(),
            dedup_key("https://e.com/b").unwrap()
        );
    }

    #[test]
    fn key_is_32_bytes_and_hex_is_64_chars() {
        let key = dedup_key("https://e.com/").unwrap();
        assert_eq!(key.as_bytes().len(), 32);
        assert_eq!(key.to_hex().len(), 64);
    }

    #[test]
    fn invalid_url_is_an_error() {
        assert!(matches!(
            dedup_key("ftp://e.com/x"),
            Err(FrontierError::InvalidUrl(_))
        ));
        assert!(matches!(
            dedup_key("not a url"),
            Err(FrontierError::InvalidUrl(_))
        ));
    }
}
