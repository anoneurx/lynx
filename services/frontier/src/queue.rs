//! The frontier queue: the trait the crawler's worker loop claims against, and the
//! in-memory reference implementation used in tests.
//!
//! The trait is deliberately small and object-safe. Postgres (`SELECT … FOR UPDATE
//! SKIP LOCKED`) is the production store; the in-memory implementation here exists
//! so the scheduling, dedup, and priority rules can be tested without a database.
//!
//! Every method is asynchronous but returns immediately in the in-memory case. The
//! boxed-future return keeps the trait `dyn`-compatible without pulling in a
//! procedural-macro dependency.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use lynx_common::ids::JobId;
use lynx_common::time::Clock;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::dedup::{self, DedupKey};
use crate::error::FrontierError;
use crate::priority::{self, PriorityInputs, PriorityWeights};

/// A boxed, `Send` future. A stand-in for `async fn` in traits that stays
/// `dyn`-compatible on stable Rust without an extra crate.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How a URL was discovered. Mirrors the Postgres `discovery_source` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Discovery {
    /// Found on a crawled page.
    Outlink,
    /// Found in a sitemap.
    Sitemap,
    /// Found in an RSS/Atom feed.
    Feed,
    /// A configured seed.
    Seed,
    /// Submitted by an operator.
    Operator,
}

/// A URL offered to the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFrontierItem {
    /// The URL; it is canonicalised before dedup, so it need not be canonical here.
    pub url: String,
    /// The host, for per-host politeness.
    pub host: String,
    /// Link depth from a seed.
    pub depth: u16,
    /// Discovery-hint component in `[0,1]`.
    pub hint: f64,
    /// Staleness component in `[0,1]` (1.0 if never crawled).
    pub staleness: f64,
    /// Freshness component in `[0,1]`.
    pub freshness: f64,
    /// How it was discovered.
    pub discovery: Discovery,
    /// Earliest claim time (immediate if `UNIX_EPOCH`).
    pub not_before: OffsetDateTime,
}

impl NewFrontierItem {
    /// An ordinary outlink: no hint, no freshness, immediately claimable.
    #[must_use]
    pub fn outlink(url: impl Into<String>, host: impl Into<String>, depth: u16) -> Self {
        Self {
            url: url.into(),
            host: host.into(),
            depth,
            hint: 0.0,
            staleness: 0.0,
            freshness: 0.0,
            discovery: Discovery::Outlink,
            not_before: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// A seed URL: maximum hint, depth zero, immediately claimable.
    #[must_use]
    pub fn seed(url: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            host: host.into(),
            depth: 0,
            hint: 1.0,
            staleness: 1.0,
            freshness: 0.0,
            discovery: Discovery::Seed,
            not_before: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// A sitemap entry, carrying a freshness signal in `[0,1]`.
    #[must_use]
    pub fn sitemap(url: impl Into<String>, host: impl Into<String>, freshness: f64) -> Self {
        Self {
            url: url.into(),
            host: host.into(),
            depth: 1,
            hint: 0.5,
            staleness: 0.0,
            freshness: freshness.clamp(0.0, 1.0),
            discovery: Discovery::Sitemap,
            not_before: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// Set the earliest claim time.
    #[must_use]
    pub fn not_before(mut self, at: OffsetDateTime) -> Self {
        self.not_before = at;
        self
    }

    /// Set the discovery hint.
    #[must_use]
    pub fn with_hint(mut self, hint: f64) -> Self {
        self.hint = hint;
        self
    }

    /// Set the staleness signal.
    #[must_use]
    pub fn with_staleness(mut self, staleness: f64) -> Self {
        self.staleness = staleness;
        self
    }
}

/// The result of an enqueue: either a new entry or a duplicate of one already queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    /// A new entry was created.
    Enqueued(JobId),
    /// The canonical URL was already queued; the existing entry's id is returned.
    Duplicate(JobId),
}

impl EnqueueOutcome {
    /// The id of the entry, whether new or existing.
    #[must_use]
    pub fn id(self) -> JobId {
        match self {
            Self::Enqueued(id) | Self::Duplicate(id) => id,
        }
    }
}

/// A URL claimed by a worker.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedItem {
    /// The frontier entry id.
    pub id: JobId,
    /// The URL to fetch.
    pub url: String,
    /// The host, for politeness bookkeeping.
    pub host: String,
    /// Link depth.
    pub depth: u16,
    /// How it was discovered.
    pub discovery: Discovery,
    /// Attempts made so far, including this one.
    pub attempts: u32,
}

/// The crawl frontier.
pub trait FrontierQueue: Send + Sync {
    /// Offer a URL. A URL whose canonical form is already queued is a duplicate.
    fn enqueue(
        &self,
        item: NewFrontierItem,
    ) -> BoxFuture<'_, Result<EnqueueOutcome, FrontierError>>;

    /// Claim up to `limit` due entries for `worker`, highest priority first.
    fn claim<'a>(
        &'a self,
        worker: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<ClaimedItem>, FrontierError>>;

    /// Mark an entry as successfully processed and free its dedup key.
    fn complete(&self, id: JobId) -> BoxFuture<'_, Result<(), FrontierError>>;

    /// Return an entry to the queue, claimable no earlier than `not_before`.
    fn retry(
        &self,
        id: JobId,
        not_before: OffsetDateTime,
    ) -> BoxFuture<'_, Result<(), FrontierError>>;

    /// Mark an entry as denied and free its dedup key.
    fn deny(&self, id: JobId) -> BoxFuture<'_, Result<(), FrontierError>>;

    /// Number of entries still awaiting a terminal outcome.
    fn len(&self) -> usize;

    /// Whether the queue holds no live entries.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QState {
    Ready,
    Claimed,
    Done,
    Denied,
}

#[derive(Debug, Clone)]
struct Entry {
    id: JobId,
    key: DedupKey,
    url: String,
    host: String,
    depth: u16,
    hint: f64,
    staleness: f64,
    freshness: f64,
    discovery: Discovery,
    enqueued_at: OffsetDateTime,
    not_before: OffsetDateTime,
    attempts: u32,
    state: QState,
    claimed_by: Option<String>,
}

#[derive(Debug, Default)]
struct Inner {
    by_key: HashMap<DedupKey, JobId>,
    items: HashMap<JobId, Entry>,
}

/// In-memory reference frontier. Used in tests and local runs; not durable.
#[derive(Debug)]
pub struct InMemoryFrontier {
    clock: Arc<dyn Clock>,
    weights: PriorityWeights,
    inner: Mutex<Inner>,
}

impl InMemoryFrontier {
    /// Create a frontier driven by `clock` with default priority weights.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_weights(clock, PriorityWeights::default())
    }

    /// Create a frontier with explicit priority weights.
    #[must_use]
    pub fn with_weights(clock: Arc<dyn Clock>, weights: PriorityWeights) -> Self {
        Self {
            clock,
            weights,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, FrontierError> {
        self.inner.lock().map_err(|_| FrontierError::Poisoned)
    }

    fn effective_priority(&self, entry: &Entry, now: OffsetDateTime) -> f64 {
        priority::compute(
            &self.weights,
            &PriorityInputs {
                depth: entry.depth,
                hint: entry.hint,
                staleness: entry.staleness,
                freshness: entry.freshness,
                enqueued_at: entry.enqueued_at,
                now,
            },
        )
    }
}

impl FrontierQueue for InMemoryFrontier {
    fn enqueue(
        &self,
        item: NewFrontierItem,
    ) -> BoxFuture<'_, Result<EnqueueOutcome, FrontierError>> {
        Box::pin(async move {
            let key = dedup::dedup_key(&item.url)?;
            let now = self.clock.now();
            let mut inner = self.lock()?;

            if let Some(existing) = inner.by_key.get(&key) {
                return Ok(EnqueueOutcome::Duplicate(*existing));
            }

            let id = JobId::new();
            inner.items.insert(
                id,
                Entry {
                    id,
                    key,
                    url: item.url,
                    host: item.host,
                    depth: item.depth,
                    hint: item.hint,
                    staleness: item.staleness,
                    freshness: item.freshness,
                    discovery: item.discovery,
                    enqueued_at: now,
                    not_before: item.not_before,
                    attempts: 0,
                    state: QState::Ready,
                    claimed_by: None,
                },
            );
            inner.by_key.insert(key, id);
            Ok(EnqueueOutcome::Enqueued(id))
        })
    }

    fn claim<'a>(
        &'a self,
        worker: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<ClaimedItem>, FrontierError>> {
        Box::pin(async move {
            let now = self.clock.now();
            let mut inner = self.lock()?;

            let mut ready: Vec<&Entry> = inner
                .items
                .values()
                .filter(|e| e.state == QState::Ready && e.not_before <= now)
                .collect();

            ready.sort_by(|a, b| {
                self.effective_priority(b, now)
                    .total_cmp(&self.effective_priority(a, now))
                    .then_with(|| a.not_before.cmp(&b.not_before))
                    .then_with(|| a.enqueued_at.cmp(&b.enqueued_at))
                    .then_with(|| a.id.cmp(&b.id))
            });

            let chosen: Vec<JobId> = ready.iter().take(limit).map(|e| e.id).collect();
            let mut claimed = Vec::with_capacity(chosen.len());
            for id in chosen {
                if let Some(entry) = inner.items.get_mut(&id) {
                    entry.state = QState::Claimed;
                    entry.attempts += 1;
                    entry.claimed_by = Some(worker.to_string());
                    claimed.push(ClaimedItem {
                        id: entry.id,
                        url: entry.url.clone(),
                        host: entry.host.clone(),
                        depth: entry.depth,
                        discovery: entry.discovery,
                        attempts: entry.attempts,
                    });
                }
            }
            Ok(claimed)
        })
    }

    fn complete(&self, id: JobId) -> BoxFuture<'_, Result<(), FrontierError>> {
        Box::pin(async move { self.finish(id, QState::Done) })
    }

    fn deny(&self, id: JobId) -> BoxFuture<'_, Result<(), FrontierError>> {
        Box::pin(async move { self.finish(id, QState::Denied) })
    }

    fn retry(
        &self,
        id: JobId,
        not_before: OffsetDateTime,
    ) -> BoxFuture<'_, Result<(), FrontierError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let entry = inner
                .items
                .get_mut(&id)
                .ok_or(FrontierError::NotFound(id.as_uuid()))?;
            entry.state = QState::Ready;
            entry.not_before = not_before;
            entry.claimed_by = None;
            Ok(())
        })
    }

    fn len(&self) -> usize {
        self.lock().map_or(0, |inner| {
            inner
                .items
                .values()
                .filter(|e| matches!(e.state, QState::Ready | QState::Claimed))
                .count()
        })
    }
}

impl InMemoryFrontier {
    fn finish(&self, id: JobId, terminal: QState) -> Result<(), FrontierError> {
        let mut inner = self.lock()?;
        let key = {
            let entry = inner
                .items
                .get_mut(&id)
                .ok_or(FrontierError::NotFound(id.as_uuid()))?;
            entry.state = terminal;
            entry.claimed_by = None;
            entry.key
        };
        // A terminal entry no longer blocks a future re-enqueue of the same URL.
        inner.by_key.remove(&key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lynx_common::time::FixedClock;
    use pretty_assertions::assert_eq;
    use time::macros::datetime;
    use time::Duration;

    fn frontier() -> (InMemoryFrontier, FixedClock) {
        let clock = FixedClock::at(datetime!(2026-02-18 12:00 UTC));
        (InMemoryFrontier::new(Arc::new(clock.clone())), clock)
    }

    #[tokio::test]
    async fn enqueue_then_claim() {
        let (q, _clock) = frontier();
        let outcome = q
            .enqueue(NewFrontierItem::seed("https://e.com/", "e.com"))
            .await;
        assert!(matches!(outcome, Ok(EnqueueOutcome::Enqueued(_))));

        let claimed = q.claim("w1", 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].url, "https://e.com/");
        assert_eq!(claimed[0].attempts, 1);
        assert_eq!(q.len(), 1, "claimed but not yet terminal");
    }

    #[tokio::test]
    async fn duplicate_urls_are_not_double_queued() {
        let (q, _clock) = frontier();
        let a = q
            .enqueue(NewFrontierItem::outlink(
                "https://e.com/p?utm_source=x",
                "e.com",
                1,
            ))
            .await
            .unwrap();
        let b = q
            .enqueue(NewFrontierItem::outlink("https://e.com/p", "e.com", 1))
            .await
            .unwrap();
        assert!(matches!(b, EnqueueOutcome::Duplicate(_)));
        assert_eq!(a.id(), b.id());
        assert_eq!(q.len(), 1);
    }

    #[tokio::test]
    async fn higher_priority_is_claimed_first() {
        let (q, _clock) = frontier();
        q.enqueue(NewFrontierItem::outlink("https://e.com/a", "e.com", 3))
            .await
            .unwrap();
        let seed = q
            .enqueue(NewFrontierItem::seed("https://e.com/s", "e.com"))
            .await
            .unwrap();
        let claimed = q.claim("w1", 1).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, seed.id());
    }

    #[tokio::test]
    async fn future_entries_are_not_claimed_until_due() {
        let (q, clock) = frontier();
        let later = clock.now() + Duration::hours(1);
        q.enqueue(NewFrontierItem::seed("https://e.com/later", "e.com").not_before(later))
            .await
            .unwrap();
        assert!(q.claim("w1", 10).await.unwrap().is_empty());
        clock.advance(Duration::hours(2));
        assert_eq!(q.claim("w1", 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn claim_respects_the_limit() {
        let (q, _clock) = frontier();
        for i in 0..5 {
            q.enqueue(NewFrontierItem::outlink(
                format!("https://e.com/{i}"),
                "e.com",
                1,
            ))
            .await
            .unwrap();
        }
        assert_eq!(q.claim("w1", 2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn completing_frees_the_dedup_key() {
        let (q, _clock) = frontier();
        let first = q
            .enqueue(NewFrontierItem::seed("https://e.com/", "e.com"))
            .await
            .unwrap();
        q.complete(first.id()).await.unwrap();
        assert_eq!(q.len(), 0);

        let again = q
            .enqueue(NewFrontierItem::seed("https://e.com/", "e.com"))
            .await
            .unwrap();
        assert!(matches!(again, EnqueueOutcome::Enqueued(_)));
        assert_ne!(again.id(), first.id());
    }

    #[tokio::test]
    async fn retry_makes_an_entry_claimable_again() {
        let (q, clock) = frontier();
        let item = q
            .enqueue(NewFrontierItem::seed("https://e.com/", "e.com"))
            .await
            .unwrap();
        let claimed = q.claim("w1", 1).await.unwrap();
        assert_eq!(claimed[0].attempts, 1);

        q.retry(item.id(), clock.now()).await.unwrap();
        let reclaimed = q.claim("w1", 1).await.unwrap();
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].attempts, 2);
    }

    #[tokio::test]
    async fn terminal_entries_are_not_counted_as_live() {
        let (q, _clock) = frontier();
        let a = q
            .enqueue(NewFrontierItem::seed("https://e.com/a", "e.com"))
            .await
            .unwrap();
        let b = q
            .enqueue(NewFrontierItem::seed("https://e.com/b", "e.com"))
            .await
            .unwrap();
        q.complete(a.id()).await.unwrap();
        q.deny(b.id()).await.unwrap();
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
    }
}
