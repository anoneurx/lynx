//! Stable hashing and HMAC helpers.
//!
//! Used for cache keys, dedup signatures, and rate-limit buckets. Cache keys are
//! HMACs (keyed), never plain hashes, so a cached response cannot be reversed into a
//! query by anyone who can read the cache.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// A BLAKE3 digest of content, used for dedup and integrity. Lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(String);

impl ContentHash {
    /// Hash a byte slice.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(hex::encode(blake3::hash(bytes).as_bytes()))
    }

    /// The hex representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A keyed message authentication code (HMAC-SHA256). Lowercase hex.
///
/// Used for cache keys and rotating rate-limit buckets. Never store the key with the
/// output; the key lives only in memory or a secret store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hmac256(String);

impl Hmac256 {
    /// Compute `HMAC-SHA256(key, message)`.
    ///
    /// # Panics
    /// Never in practice: HMAC accepts a key of any length, so the fallback used to keep
    /// this function free of `unwrap` is unreachable.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn mac(key: &[u8], message: &[u8]) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(key)
            .unwrap_or_else(|_| Hmac::<Sha256>::new_from_slice(&[]).expect("empty key is valid"));
        mac.update(message);
        Self(hex::encode(mac.finalize().into_bytes()))
    }

    /// Hex representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// First `n` bytes of the MAC, hex-encoded. Useful for compact bucket keys.
    #[must_use]
    pub fn truncated(&self, n: usize) -> String {
        let full = &self.0;
        let take = (n * 2).min(full.len());
        full[..take].to_string()
    }
}

impl std::fmt::Display for Hmac256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Compare two byte slices in constant time. Use for secrets and tokens.
#[must_use]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_hex() {
        let a = ContentHash::of(b"hello");
        let b = ContentHash::of(b"hello");
        let c = ContentHash::of(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.as_str().len(), 64);
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hmac_is_keyed() {
        let a = Hmac256::mac(b"key1", b"message");
        let b = Hmac256::mac(b"key2", b"message");
        let c = Hmac256::mac(b"key1", b"message");
        assert_ne!(a, b, "different keys must give different MACs");
        assert_eq!(a, c);
    }

    #[test]
    fn truncated_mac_has_expected_length() {
        let mac = Hmac256::mac(b"k", b"m");
        assert_eq!(mac.truncated(3).len(), 6);
    }

    #[test]
    fn constant_time_equality_matches_normal_equality() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
