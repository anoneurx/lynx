//! Host-keyed caching of parsed `robots.txt`.
//!
//! Fetching and parsing `robots.txt` on every URL would double our request rate and
//! hand a host an easy way to amplify load. Entries are keyed by host and expire on a
//! TTL; the network fetch that produces an entry lives in the downloader (Step 11),
//! so this crate only defines the contract and an in-memory implementation.
//!
//! Negative results are cached too: a 404 is remembered as "allow all" rather than
//! re-fetched for every URL on the host.

use std::collections::HashMap;
use std::sync::Mutex;

use time::{Duration, OffsetDateTime};

use crate::robots::{
    effective_crawl_delay_seconds, policy_for_status, GroupSelection, RobotsDecision, RobotsRules,
    RobotsStatusPolicy,
};

/// A parsed `robots.txt` together with its freshness window.
#[derive(Debug, Clone)]
pub struct CachedRobots {
    rules: RobotsRules,
    fetched_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    truncated: bool,
}

impl CachedRobots {
    /// Build a cache entry that is fresh until `now + ttl`.
    #[must_use]
    pub fn new(rules: RobotsRules, truncated: bool, now: OffsetDateTime, ttl: Duration) -> Self {
        Self {
            rules,
            fetched_at: now,
            expires_at: now + ttl,
            truncated,
        }
    }

    /// Whether the entry may still be used at `now`.
    #[must_use]
    pub fn is_fresh(&self, now: OffsetDateTime) -> bool {
        now < self.expires_at
    }

    /// When the entry was fetched.
    #[must_use]
    pub fn fetched_at(&self) -> OffsetDateTime {
        self.fetched_at
    }

    /// Whether the source file was truncated at the byte cap.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// The underlying parsed rules.
    #[must_use]
    pub fn rules(&self) -> &RobotsRules {
        &self.rules
    }

    /// Select the rules for `user_agent`.
    #[must_use]
    pub fn select(&self, user_agent: &str) -> GroupSelection {
        self.rules.select(user_agent)
    }

    /// Decide a path for `user_agent`.
    #[must_use]
    pub fn decision(&self, user_agent: &str, path: &str) -> RobotsDecision {
        self.rules.select(user_agent).decision(path)
    }

    /// The effective crawl delay for `user_agent`, never below `min_inter_request`.
    #[must_use]
    pub fn crawl_delay_seconds(&self, user_agent: &str, min_inter_request: u64) -> u64 {
        effective_crawl_delay_seconds(
            self.rules.select(user_agent).crawl_delay_seconds(),
            min_inter_request,
        )
    }
}

/// Build a cache entry from a non-2xx `robots.txt` status.
///
/// `Parse` returns `None` because only the caller has the body; `Cooldown` and
/// `DisallowAllRetry` are transient and must not be frozen into a policy.
#[must_use]
pub fn cached_from_status(status: u16, now: OffsetDateTime, ttl: Duration) -> Option<CachedRobots> {
    match policy_for_status(status) {
        RobotsStatusPolicy::Parse
        | RobotsStatusPolicy::Cooldown
        | RobotsStatusPolicy::DisallowAllRetry => None,
        RobotsStatusPolicy::AllowAll => {
            Some(CachedRobots::new(RobotsRules::allow_all(), false, now, ttl))
        }
        RobotsStatusPolicy::DisallowAll => Some(CachedRobots::new(
            RobotsRules::disallow_all(),
            false,
            now,
            ttl,
        )),
    }
}

/// A host-keyed store of parsed `robots.txt`.
///
/// Implementations must be safe to share across the crawler's tasks. The production
/// store is shared across workers; [`InMemoryRobotsCache`] is the single-process
/// implementation used for tests and small deployments.
pub trait RobotsCache: std::fmt::Debug + Send + Sync {
    /// Return a fresh entry for `host`, or `None` if absent or expired.
    fn get(&self, host: &str, now: OffsetDateTime) -> Option<CachedRobots>;

    /// Insert or replace the entry for `host`.
    fn put(&self, host: &str, robots: CachedRobots);

    /// Drop the entry for `host`, forcing the next lookup to re-fetch.
    fn invalidate(&self, host: &str);
}

/// An in-memory [`RobotsCache`].
#[derive(Debug)]
pub struct InMemoryRobotsCache {
    entries: Mutex<HashMap<String, CachedRobots>>,
}

impl InMemoryRobotsCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The number of stored entries, including expired ones not yet evicted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |guard| guard.len())
    }

    /// Whether the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for InMemoryRobotsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RobotsCache for InMemoryRobotsCache {
    fn get(&self, host: &str, now: OffsetDateTime) -> Option<CachedRobots> {
        let guard = self.entries.lock().ok()?;
        let entry = guard.get(host)?;
        entry.is_fresh(now).then(|| entry.clone())
    }

    fn put(&self, host: &str, robots: CachedRobots) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.insert(host.to_string(), robots);
        }
    }

    fn invalidate(&self, host: &str) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.remove(host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robots::{RobotsRules, USER_AGENT_GROUP};
    use time::macros::datetime;

    fn ttl() -> Duration {
        Duration::hours(1)
    }

    #[test]
    fn fresh_entry_is_returned_then_expires() {
        let cache = InMemoryRobotsCache::new();
        let t0 = datetime!(2026-02-18 09:00 UTC);
        let rules = RobotsRules::parse("User-agent: *\nDisallow: /x\n");
        cache.put("example.com", CachedRobots::new(rules, false, t0, ttl()));

        assert!(cache
            .get("example.com", t0 + Duration::minutes(30))
            .is_some());
        // At exactly the expiry instant the entry is stale.
        assert!(cache.get("example.com", t0 + ttl()).is_none());
    }

    #[test]
    fn invalidate_forces_a_refetch() {
        let cache = InMemoryRobotsCache::new();
        let t0 = datetime!(2026-02-18 09:00 UTC);
        cache.put(
            "example.com",
            CachedRobots::new(RobotsRules::allow_all(), false, t0, ttl()),
        );
        cache.invalidate("example.com");
        assert!(cache.get("example.com", t0).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn missing_host_is_none() {
        let cache = InMemoryRobotsCache::new();
        assert!(cache
            .get("nope.test", datetime!(2026-02-18 09:00 UTC))
            .is_none());
    }

    #[test]
    fn negative_statuses_become_entries() {
        let t0 = datetime!(2026-02-18 09:00 UTC);
        let allow = cached_from_status(404, t0, ttl()).unwrap();
        assert_eq!(
            allow.decision(USER_AGENT_GROUP, "/any"),
            RobotsDecision::Allow
        );

        let deny = cached_from_status(403, t0, ttl()).unwrap();
        assert_eq!(
            deny.decision(USER_AGENT_GROUP, "/any"),
            RobotsDecision::Disallow
        );

        assert!(cached_from_status(200, t0, ttl()).is_none());
        assert!(cached_from_status(429, t0, ttl()).is_none());
        assert!(cached_from_status(503, t0, ttl()).is_none());
    }

    #[test]
    fn crawl_delay_respects_the_floor() {
        let t0 = datetime!(2026-02-18 09:00 UTC);
        let rules = RobotsRules::parse("User-agent: *\nCrawl-delay: 2\n");
        let entry = CachedRobots::new(rules, false, t0, ttl());
        assert_eq!(entry.crawl_delay_seconds(USER_AGENT_GROUP, 5), 5);
        assert_eq!(entry.crawl_delay_seconds(USER_AGENT_GROUP, 1), 2);
    }
}
