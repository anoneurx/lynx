//! Testable time access and bucketing.
//!
//! All time-dependent logic (freshness, retention, rate-limit salt rotation) takes a
//! [`Clock`] so it is deterministic under test. `SystemTime::now()` is confined to
//! [`SystemClock`].

use std::sync::Arc;

use time::{Date, Duration, OffsetDateTime};

/// A source of the current time. Deterministic implementations are used in tests.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The current instant, UTC.
    fn now(&self) -> OffsetDateTime;
}

/// The real clock. The only place `SystemTime`/`OffsetDateTime::now_utc` is called.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// A fixed clock for tests. Advance it with [`FixedClock::advance`].
#[derive(Debug, Clone)]
pub struct FixedClock {
    now: Arc<std::sync::Mutex<OffsetDateTime>>,
}

impl FixedClock {
    /// Create a clock frozen at `now`.
    #[must_use]
    pub fn at(now: OffsetDateTime) -> Self {
        Self {
            now: Arc::new(std::sync::Mutex::new(now)),
        }
    }

    /// Advance the clock by `delta`.
    pub fn advance(&self, delta: Duration) {
        if let Ok(mut guard) = self.now.lock() {
            *guard += delta;
        }
    }

    /// Set the clock to `now`.
    pub fn set(&self, now: OffsetDateTime) {
        if let Ok(mut guard) = self.now.lock() {
            *guard = now;
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        self.now
            .lock()
            .map_or_else(|_| OffsetDateTime::UNIX_EPOCH, |guard| *guard)
    }
}

/// The number of whole days between two dates, negative if `to < from`.
#[must_use]
pub fn days_between(from: Date, to: Date) -> i64 {
    (to - from).whole_days()
}

/// Floor an instant to the start of its UTC day. Used by daily-salt rotation and
/// day-bucketed rate limits.
#[must_use]
pub fn start_of_day(t: OffsetDateTime) -> OffsetDateTime {
    t.replace_hour(0)
        .unwrap_or(t)
        .replace_minute(0)
        .unwrap_or(t)
        .replace_second(0)
        .unwrap_or(t)
        .replace_nanosecond(0)
        .unwrap_or(t)
}

/// A calendar key `YYYY-MM-DD`, used for daily-rotating salts.
#[must_use]
pub fn day_key(t: OffsetDateTime) -> String {
    let d = t.date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn fixed_clock_advances() {
        let clock = FixedClock::at(datetime!(2026-02-18 09:00 UTC));
        assert_eq!(clock.now(), datetime!(2026-02-18 09:00 UTC));
        clock.advance(Duration::hours(25));
        assert_eq!(clock.now(), datetime!(2026-02-19 10:00 UTC));
    }

    #[test]
    fn day_key_is_zero_padded() {
        assert_eq!(day_key(datetime!(2026-02-03 23:59 UTC)), "2026-02-03");
    }

    #[test]
    fn start_of_day_truncates_time() {
        let t = start_of_day(datetime!(2026-02-18 23:59:59.5 UTC));
        assert_eq!(t, datetime!(2026-02-18 00:00 UTC));
    }

    #[test]
    fn days_between_is_signed() {
        let a = datetime!(2026-01-01 00:00 UTC).date();
        let b = datetime!(2026-01-11 00:00 UTC).date();
        assert_eq!(days_between(a, b), 10);
        assert_eq!(days_between(b, a), -10);
    }

    #[test]
    fn salts_rotate_between_days() {
        let d1 = day_key(datetime!(2026-02-18 00:00 UTC));
        let d2 = day_key(datetime!(2026-02-18 23:59 UTC));
        let d3 = day_key(datetime!(2026-02-19 00:00 UTC));
        assert_eq!(d1, d2);
        assert_ne!(d2, d3);
    }
}
