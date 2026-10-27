//! Runtime controls on the answer plane: kill switch, token budget, circuit
//! breaker, and the answer cache.
//!
//! Spec §Q controls, all process-local (v0.1 runs a single answer binary per
//! replica; cross-replica budget is a Redis counter later — the honest statement
//! is in docs/ai/astra.md). All timing flows through an injected
//! [`lynx_common::time::Clock`] so every behaviour here is testable
//! deterministically.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use lynx_common::time::Clock;
use time::OffsetDateTime;

/// A runtime kill switch (§Q). Enabled via config, but flippable at run time —
/// an operator cuts the answer plane off the same way they enabled it: no restart.
#[derive(Debug, Default)]
pub struct KillSwitch {
    tripped: AtomicBool,
    reason: Mutex<Option<String>>,
}

impl KillSwitch {
    /// A clean switch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether answering is cut off.
    #[must_use]
    pub fn is_tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }

    /// Trip the switch; subsequent requests are refused with `reason`.
    pub fn trip(&self, reason: impl Into<String>) {
        self.reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(reason.into());
        self.tripped.store(true, Ordering::Relaxed);
    }

    /// Clear the switch.
    pub fn reset(&self) {
        self.tripped.store(false, Ordering::Relaxed);
        *self
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// The reason recorded at trip time, if any.
    #[must_use]
    pub fn trip_reason(&self) -> Option<String> {
        self.reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// The daily token budget (§Q: prompt + completion tokens counted by the
/// provider response). In-memory and per-replica; saturating.
#[derive(Debug)]
pub struct TokenBudget {
    /// Budget per UTC day.
    daily_limit: u64,
    consumed: AtomicU64,
    consumed_day: Mutex<String>,
}

impl TokenBudget {
    /// A budget of `daily_limit` tokens per UTC day.
    #[must_use]
    pub fn new(daily_limit: u64) -> Self {
        Self {
            daily_limit,
            consumed: AtomicU64::new(0),
            consumed_day: Mutex::new(String::new()),
        }
    }

    /// Remaining tokens today; the day boundary rolls up on read.
    #[must_use]
    pub fn remaining(&self, clock: &dyn Clock) -> u64 {
        self.ensure_day(clock);
        let used = self.consumed.load(Ordering::Relaxed);
        self.daily_limit.saturating_sub(used)
    }

    /// Record usage (the provider-reported total, even when zero).
    ///
    /// Saturating, via a compare-exchange loop: `fetch_add` wraps, and a
    /// budget that wraps after an oversized charge is a hole, not a bound.
    pub fn charge(&self, clock: &dyn Clock, tokens: u64) {
        self.ensure_day(clock);
        let mut current = self.consumed.load(Ordering::Relaxed);
        loop {
            let next = current.saturating_add(tokens);
            match self.consumed.compare_exchange(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }

    /// Whether the budget is exhausted for the clock's current day.
    #[must_use]
    pub fn is_exhausted(&self, clock: &dyn Clock) -> bool {
        self.remaining(clock) == 0
    }

    fn ensure_day(&self, clock: &dyn Clock) {
        let today = lynx_common::time::day_key(clock.now());
        let mut day = self
            .consumed_day
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *day != today {
            *day = today;
            self.consumed.store(0, Ordering::Relaxed);
        }
    }
}

/// A circuit breaker (§Q) over provider calls: `threshold` consecutive failures
/// open the circuit for a cooldown; a success closes it.
#[derive(Debug)]
pub struct CircuitBreaker {
    threshold: u32,
    cooldown_seconds: i64,
    consecutive_failures: AtomicU32,
    open_until: Mutex<Option<OffsetDateTime>>,
}

impl CircuitBreaker {
    /// A breaker opening after `threshold` consecutive failures for
    /// `cooldown_seconds`.
    #[must_use]
    pub fn new(threshold: u32, cooldown_seconds: i64) -> Self {
        Self {
            threshold,
            cooldown_seconds,
            consecutive_failures: AtomicU32::new(0),
            open_until: Mutex::new(None),
        }
    }

    /// Whether a call may be made right now.
    #[must_use]
    pub fn allows(&self, clock: &dyn Clock) -> bool {
        let until = *self
            .open_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match until {
            Some(t) => clock.now() >= t,
            None => true,
        }
    }

    /// Record a failure; opens the circuit at the threshold.
    pub fn record_failure(&self, clock: &dyn Clock) {
        let n = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= self.threshold {
            *self
                .open_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(clock.now() + time::Duration::seconds(self.cooldown_seconds));
        }
    }

    /// Record a success; closes the circuit and resets the counter.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        *self
            .open_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// The answer cache (§Q): hashed keys, TTL from config, in-memory.
#[derive(Debug)]
pub struct AnswerCache {
    ttl_seconds: i64,
    entries: Mutex<HashMap<String, (OffsetDateTime, String)>>,
}

impl AnswerCache {
    /// A cache with `ttl_seconds` expiry.
    #[must_use]
    pub fn new(ttl_seconds: i64) -> Self {
        Self {
            ttl_seconds,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The deterministic cache key for a question, a model, and the exact set
    /// of source doc ids in order (so a ranking change legitimately busts the
    /// cache).
    #[must_use]
    pub fn key(question: &str, model: &str, doc_ids: &[String]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(question.as_bytes());
        hasher.update(&[0]);
        hasher.update(model.as_bytes());
        for id in doc_ids {
            hasher.update(id.as_bytes());
            hasher.update(&[0]);
        }
        hasher.finalize().to_hex().to_string()
    }

    /// Put an answer body.
    pub fn put(&self, clock: &dyn Clock, key: &str, body: &str) {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.to_string(), (clock.now(), body.to_string()));
    }

    /// Get an unexpired body.
    #[must_use]
    pub fn get(&self, clock: &dyn Clock, key: &str) -> Option<String> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = clock.now();
        entries
            .get(key)
            .filter(|(written_at, _)| now < *written_at + time::Duration::seconds(self.ttl_seconds))
            .map(|(_, body)| body.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lynx_common::time::FixedClock;
    use time::macros::datetime;

    fn clock() -> FixedClock {
        FixedClock::at(datetime!(2026-03-01 00:00 UTC))
    }

    #[test]
    fn kill_switch_refuses_until_reset() {
        let switch = KillSwitch::new();
        assert!(!switch.is_tripped());
        switch.trip("testing");
        assert!(switch.is_tripped());
        assert_eq!(switch.trip_reason(), Some("testing".into()));
        switch.reset();
        assert!(!switch.is_tripped());
        assert_eq!(switch.trip_reason(), None);
    }

    #[test]
    fn budget_resets_at_the_day_boundary() {
        let clock = clock();
        let budget = TokenBudget::new(100);
        budget.charge(&clock, 90);
        assert_eq!(budget.remaining(&clock), 10);
        assert!(!budget.is_exhausted(&clock));

        budget.charge(&clock, 10);
        assert!(budget.is_exhausted(&clock));

        clock.advance(time::Duration::days(1));
        assert_eq!(budget.remaining(&clock), 100, "a new day must reset");
    }

    #[test]
    fn budget_saturates_never_overflows() {
        let clock = clock();
        let budget = TokenBudget::new(100);
        budget.charge(&clock, u64::MAX);
        assert_eq!(budget.remaining(&clock), 0);
    }

    #[test]
    fn the_breaker_opens_after_threshold_and_recovers_after_success() {
        let clock = clock();
        let breaker = CircuitBreaker::new(2, 60);
        assert!(breaker.allows(&clock));
        breaker.record_failure(&clock);
        assert!(breaker.allows(&clock), "below threshold");
        breaker.record_failure(&clock);
        assert!(!breaker.allows(&clock), "threshold reached");

        clock.advance(time::Duration::seconds(61));
        assert!(breaker.allows(&clock), "cooldown expired");
        breaker.record_success();
        breaker.record_failure(&clock);
        assert!(breaker.allows(&clock), "failures restarted after a success");
    }

    #[test]
    fn the_cache_is_deterministic_per_question_model_and_doc_set() {
        let key = AnswerCache::key("q", "m", &["d1".into(), "d2".into()]);
        assert_eq!(key, AnswerCache::key("q", "m", &["d1".into(), "d2".into()]));
        assert_ne!(key, AnswerCache::key("q2", "m", &["d1".into()]));
        assert_ne!(
            key,
            AnswerCache::key("q", "m", &["d2".into(), "d1".into()]),
            "doc set order is part of the identity"
        );
    }

    #[test]
    fn the_cache_respects_ttl_and_a_ranking_change() {
        let clock = clock();
        let cache = AnswerCache::new(600);
        let key = AnswerCache::key("q", "m", &["d1".into()]);
        cache.put(&clock, &key, "body");
        assert_eq!(cache.get(&clock, &key).as_deref(), Some("body"));

        assert_eq!(
            cache.get(&clock, &AnswerCache::key("q", "m", &[])),
            None,
            "a changed doc set must not serve a cached answer grounded elsewhere"
        );

        clock.advance(time::Duration::seconds(601));
        assert_eq!(cache.get(&clock, &key), None, "expired");
    }
}
