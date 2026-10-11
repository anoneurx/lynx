//! Rotating salted prefix-bucket rate limiting.
//!
//! The bucket is derived from the client's network prefix (`/24` for IPv4, `/48` for
//! IPv6) and a daily-rotating salt, never from a durable identifier:
//!
//! ```text
//! v4: bucket = hex(HMAC-SHA256(salt_day, ip[0..3] || 0x00))
//! v6: bucket = hex(HMAC-SHA256(salt_day, ip[0..6]))
//! salt_day  = HMAC-SHA256(server_secret, "YYYY-MM-DD")
//! TTL       = 25 h, memory-only
//! ```
//!
//! The salt changes every 24 h, so yesterday's bucket for the same prefix is a
//! different value. There is nothing durable to correlate, and nothing precise enough
//! to reconstruct an address.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;

use lynx_common::hash::Hmac256;
use lynx_common::time::day_key;
use time::{Duration, OffsetDateTime};

/// Derive the salt for a given day from the server secret. Rotates at UTC midnight.
#[must_use]
pub fn daily_salt(server_secret: &[u8], now: OffsetDateTime) -> Hmac256 {
    Hmac256::mac(server_secret, day_key(now).as_bytes())
}

/// Derive the opaque bucket key for a client address.
///
/// IPv4 uses the `/24` prefix; IPv6 uses the `/48` prefix. The result leaks neither
/// the address nor continuity across a day boundary.
#[must_use]
pub fn client_bucket(ip: IpAddr, salt: &Hmac256) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            let mut buf = [0u8; 4];
            buf[..3].copy_from_slice(&o[..3]);
            buf[3] = 0x00;
            Hmac256::mac(salt.as_str().as_bytes(), &buf).truncated(16)
        }
        IpAddr::V6(v6) => {
            let o = v6.octets();
            Hmac256::mac(salt.as_str().as_bytes(), &o[..6]).truncated(16)
        }
    }
}

/// The result of a rate-limit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// Whether the request is allowed.
    pub allowed: bool,
    /// Remaining requests in the current window.
    pub remaining: u32,
    /// When the window frees up, when not allowed.
    pub retry_after: Option<Duration>,
}

/// A memory-only sliding-window limiter. State is intentionally not persisted:
/// durable rate-limit state is itself a privacy liability.
#[derive(Debug, Default)]
pub struct SlidingWindowLimiter {
    buckets: Mutex<HashMap<String, VecDeque<OffsetDateTime>>>,
}

impl SlidingWindowLimiter {
    /// Create an empty limiter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check and record a request against `key`.
    ///
    /// # Panics
    /// Never in practice; a poisoned lock is treated as an empty state.
    #[must_use]
    pub fn check(&self, key: &str, limit: u32, window: Duration, now: OffsetDateTime) -> Decision {
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = map.entry(key.to_string()).or_default();

        let cutoff = now - window;
        while entry.front().is_some_and(|t| *t <= cutoff) {
            entry.pop_front();
        }

        let count = u32::try_from(entry.len()).unwrap_or(u32::MAX);
        if count >= limit {
            let retry = entry
                .front()
                .map_or(window, |oldest| (*oldest + window) - now);
            return Decision {
                allowed: false,
                remaining: 0,
                retry_after: Some(retry.max(Duration::ZERO)),
            };
        }

        entry.push_back(now);
        Decision {
            allowed: true,
            remaining: limit - count,
            retry_after: None,
        }
    }

    /// Remove buckets whose newest entry is older than `retention`. Call periodically.
    pub fn gc(&self, now: OffsetDateTime, retention: Duration) {
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, deque| {
            deque
                .back()
                .is_some_and(|newest| now - *newest <= retention)
        });
    }

    /// Number of tracked buckets (for tests and metrics).
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Whether no buckets are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use time::macros::datetime;

    #[test]
    fn salt_rotates_daily_and_differs_by_secret() {
        let d1 = datetime!(2026-02-18 08:00 UTC);
        let d2 = datetime!(2026-02-19 08:00 UTC);
        assert_eq!(daily_salt(b"s", d1), daily_salt(b"s", d1));
        assert_ne!(daily_salt(b"s", d1), daily_salt(b"s", d2));
        assert_ne!(daily_salt(b"s", d1), daily_salt(b"t", d1));
    }

    #[test]
    fn same_v4_prefix_shares_bucket_and_different_prefix_does_not() {
        let salt = daily_salt(b"secret", datetime!(2026-02-18 00:00 UTC));
        let a = client_bucket(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), &salt);
        let b = client_bucket(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 250)), &salt);
        let c = client_bucket(IpAddr::V4(Ipv4Addr::new(203, 0, 114, 10)), &salt);
        assert_eq!(a, b, "same /24 must share a bucket");
        assert_ne!(a, c, "different /24 must differ");
    }

    #[test]
    fn v6_uses_48_bit_prefix() {
        let salt = daily_salt(b"secret", datetime!(2026-02-18 00:00 UTC));
        let a = client_bucket(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            &salt,
        );
        let b = client_bucket(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0xffff, 0, 0, 0, 2)),
            &salt,
        );
        let c = client_bucket(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb9, 0, 0, 0, 0, 0, 1)),
            &salt,
        );
        assert_eq!(a, b, "same /48 must share a bucket");
        assert_ne!(a, c);
    }

    #[test]
    fn bucket_changes_across_day_boundary() {
        let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
        let s1 = daily_salt(b"secret", datetime!(2026-02-18 23:59 UTC));
        let s2 = daily_salt(b"secret", datetime!(2026-02-19 00:01 UTC));
        assert_ne!(client_bucket(ip, &s1), client_bucket(ip, &s2));
    }

    #[test]
    fn limiter_allows_then_blocks_then_recovers() {
        let limiter = SlidingWindowLimiter::new();
        let t0 = datetime!(2026-02-18 00:00 UTC);
        let window = Duration::minutes(1);

        for i in 0..3 {
            let d = limiter.check("k", 3, window, t0 + Duration::seconds(i));
            assert!(d.allowed, "request {i} should be allowed");
        }
        let blocked = limiter.check("k", 3, window, t0 + Duration::seconds(4));
        assert!(!blocked.allowed);
        assert_eq!(blocked.remaining, 0);
        assert!(blocked.retry_after.is_some());

        // Window slides: old entries expire.
        let later = limiter.check("k", 3, window, t0 + Duration::seconds(61));
        assert!(later.allowed);
    }

    #[test]
    fn gc_drops_stale_buckets() {
        let limiter = SlidingWindowLimiter::new();
        let t0 = datetime!(2026-02-18 00:00 UTC);
        let _ = limiter.check("a", 10, Duration::minutes(1), t0);
        let _ = limiter.check("b", 10, Duration::minutes(1), t0 + Duration::hours(30));
        assert_eq!(limiter.len(), 2);
        limiter.gc(t0 + Duration::hours(30), Duration::hours(25));
        assert_eq!(limiter.len(), 1);
    }
}
