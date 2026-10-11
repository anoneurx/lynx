//! API key generation, storage, and verification.
//!
//! Keys are shown exactly once, at creation. Only a SHA-256 hash of the secret is
//! stored, and verification is constant-time. The public prefix is what appears in
//! logs and admin lists; it can never be used to authenticate.
//!
//! ```text
//! key        = "lynx_" || public_id(8) || "_" || secret(32 bytes, base64url)
//! stored     = SHA-256(secret_bytes)          (hex)
//! presented  = public_id + secret, verified in constant time
//! ```

use base64::Engine as _;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Length in bytes of the random secret.
const SECRET_BYTES: usize = 32;
/// Length in characters of the public, non-secret id prefix.
const PUBLIC_ID_LEN: usize = 8;

/// A newly generated API key. The `secret` field is shown once and must never be
/// logged or stored.
#[derive(Clone)]
pub struct NewApiKey {
    /// The full key, to present to the caller once.
    pub key: String,
    /// The public id embedded in the key, safe to log.
    pub public_id: String,
    /// The hash to persist (hex SHA-256 of the secret).
    pub hash: String,
}

impl std::fmt::Debug for NewApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewApiKey")
            .field("public_id", &self.public_id)
            .field("key", &"<redacted>")
            .field("hash", &"<redacted>")
            .finish()
    }
}

/// Generate a new key from a cryptographic RNG.
#[must_use]
pub fn generate() -> NewApiKey {
    let mut rng = rand::thread_rng();
    generate_with(&mut rng)
}

/// Generate a new key from a caller-provided RNG (for deterministic tests).
pub fn generate_with<R: RngCore>(rng: &mut R) -> NewApiKey {
    let mut secret = [0u8; SECRET_BYTES];
    rng.fill_bytes(&mut secret);
    let mut id_bytes = [0u8; PUBLIC_ID_LEN / 2];
    rng.fill_bytes(&mut id_bytes);
    // Hex, not base64url: the public id is embedded in a `_`-delimited key, so its
    // alphabet must not contain `_`.
    let public_id = hex::encode(id_bytes);
    let secret_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
    NewApiKey {
        key: format!("lynx_{public_id}_{secret_b64}"),
        hash: hash_secret(secret_b64.as_bytes()),
        public_id,
    }
}

/// Hash secret bytes for storage.
#[must_use]
pub fn hash_secret(secret: &[u8]) -> String {
    hex::encode(Sha256::digest(secret))
}

/// Split a presented key into `(public_id, secret_string)`, if it is well-formed.
#[must_use]
pub fn split(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("lynx_")?;
    let (id, secret) = rest.split_once('_')?;
    if id.len() != PUBLIC_ID_LEN || secret.is_empty() {
        return None;
    }
    Some((id, secret))
}

/// Verify a presented key against a stored hash, in constant time.
///
/// Returns `false` for malformed keys as well as wrong ones, so callers cannot
/// distinguish "malformed" from "invalid" by timing.
#[must_use]
pub fn verify(presented: &str, stored_hash_hex: &str) -> bool {
    let Some((_id, secret)) = split(presented) else {
        return false;
    };
    let computed = hash_secret(secret.as_bytes());
    let a = computed.as_bytes();
    let b = stored_hash_hex.as_bytes();
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_expected_shape_and_verifies() {
        let k = generate();
        let (id, secret) = split(&k.key).unwrap();
        assert_eq!(id, k.public_id);
        assert!(!secret.is_empty());
        assert!(verify(&k.key, &k.hash));
    }

    #[test]
    fn every_generated_key_splits_and_verifies() {
        // Regression: a base64url public id could contain `_` and break `split`.
        for _ in 0..1000 {
            let k = generate();
            let (id, secret) = split(&k.key).expect("generated key must split");
            assert_eq!(id.len(), PUBLIC_ID_LEN);
            assert!(!secret.is_empty());
            assert!(verify(&k.key, &k.hash));
        }
    }

    #[test]
    fn debug_never_reveals_secret_or_hash() {
        let k = generate();
        let dbg = format!("{k:?}");
        assert!(!dbg.contains(&k.hash));
        assert!(!dbg.contains("secret"));
    }

    #[test]
    fn wrong_or_malformed_keys_fail() {
        let k = generate();
        assert!(!verify("lynx_abcd1234_wrong", &k.hash));
        assert!(!verify("not-a-key", &k.hash));
        assert!(!verify("", &k.hash));
        assert!(!verify(&k.key, "00"));
    }

    #[test]
    fn deterministic_rng_gives_reproducible_key() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let a = generate_with(&mut rng);
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let b = generate_with(&mut rng);
        assert_eq!(a.key, b.key);
        assert_eq!(a.hash, b.hash);
    }
}
