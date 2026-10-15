//! Claim-time priority.
//!
//! Priority is a function of the URL's depth, how stale it is, whether it was
//! discovered with a strong hint (seed, sitemap, operator) versus an ordinary
//! outlink, and — crucially — how long it has waited. The age term is what stops a
//! high-volume site from starving the long tail forever; without it a crawler's
//! queue is a popularity contest and nothing old ever gets crawled.
//!
//! Weights live in configuration, not in code (see `docs/crawler/frontier.md`).

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Weights for the priority sum. Mirror of the `[crawler.priority]` config block.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PriorityWeights {
    /// Reward for being close to a seed.
    pub depth: f64,
    /// Reward for not having been crawled recently (`staleness` in `[0,1]`).
    pub staleness: f64,
    /// Reward for a strong discovery hint (`hint` in `[0,1]`).
    pub hint: f64,
    /// Reward for a promised-fresh signal such as a recent sitemap `lastmod`.
    pub freshness: f64,
    /// How much priority an entry gains per hour waited.
    pub age_boost_per_hour: f64,
    /// Ceiling on the age boost, so waiting cannot overcome everything forever.
    pub age_boost_cap: f64,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            depth: 1.0,
            staleness: 0.3,
            hint: 2.0,
            freshness: 0.2,
            age_boost_per_hour: 0.05,
            age_boost_cap: 5.0,
        }
    }
}

/// The inputs to a priority computation, other than the current instant.
///
/// `staleness`, `hint`, and `freshness` are expected in `[0,1]`; values outside the
/// range are clamped rather than rejected, because a slightly-out-of-range signal
/// should not drop a URL on the floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorityInputs {
    /// Link depth from a seed.
    pub depth: u16,
    /// Discovery hint in `[0,1]`: 1.0 for a seed/operator submission, 0.0 for an
    /// ordinary outlink.
    pub hint: f64,
    /// Staleness in `[0,1]`: 1.0 when the URL has never been crawled.
    pub staleness: f64,
    /// Promising-signal freshness in `[0,1]`.
    pub freshness: f64,
    /// When the entry entered the queue.
    pub enqueued_at: OffsetDateTime,
    /// The instant to evaluate at.
    pub now: OffsetDateTime,
}

/// The age boost alone, in priority units, capped at `weights.age_boost_cap`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // seconds and hours are well within f64's exact range
pub fn age_boost(
    weights: &PriorityWeights,
    enqueued_at: OffsetDateTime,
    now: OffsetDateTime,
) -> f64 {
    let seconds = (now - enqueued_at).whole_seconds().max(0);
    let hours = seconds as f64 / 3600.0;
    (hours * weights.age_boost_per_hour).min(weights.age_boost_cap)
}

/// Compute an entry's priority. Higher is claimed first.
#[must_use]
pub fn compute(weights: &PriorityWeights, input: &PriorityInputs) -> f64 {
    let depth_score = weights.depth / (1.0 + f64::from(input.depth));
    depth_score
        + weights.staleness * input.staleness.clamp(0.0, 1.0)
        + weights.hint * input.hint.clamp(0.0, 1.0)
        + weights.freshness * input.freshness.clamp(0.0, 1.0)
        + age_boost(weights, input.enqueued_at, input.now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use time::macros::datetime;
    use time::Duration;

    fn input(
        depth: u16,
        hint: f64,
        enqueued_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> PriorityInputs {
        PriorityInputs {
            depth,
            hint,
            staleness: 0.0,
            freshness: 0.0,
            enqueued_at,
            now,
        }
    }

    #[test]
    fn deeper_is_lower_priority() {
        let w = PriorityWeights::default();
        let now = datetime!(2026-02-18 12:00 UTC);
        let shallow = compute(&w, &input(0, 0.0, now, now));
        let deep = compute(&w, &input(5, 0.0, now, now));
        assert!(shallow > deep);
    }

    #[test]
    fn hint_beats_depth() {
        let w = PriorityWeights::default();
        let now = datetime!(2026-02-18 12:00 UTC);
        let seed = compute(&w, &input(0, 1.0, now, now));
        let outlink = compute(&w, &input(0, 0.0, now, now));
        assert!(seed > outlink);
    }

    #[test]
    fn age_boost_grows_and_is_capped() {
        let w = PriorityWeights::default();
        let enqueued = datetime!(2026-02-01 00:00 UTC);
        let one_hour = age_boost(&w, enqueued, enqueued + Duration::hours(1));
        let two_hours = age_boost(&w, enqueued, enqueued + Duration::hours(2));
        assert!(two_hours > one_hour);
        assert_eq!(one_hour, w.age_boost_per_hour);

        let ancient = age_boost(&w, enqueued, enqueued + Duration::days(3650));
        assert_eq!(ancient, w.age_boost_cap);
    }

    #[test]
    fn waiting_lets_a_long_tail_url_overtake_a_fresh_seed() {
        let w = PriorityWeights::default();
        let start = datetime!(2026-02-01 00:00 UTC);
        let old_outlink = compute(&w, &input(3, 0.0, start, start));
        // A seed discovered much later starts higher...
        let fresh_seed = compute(
            &w,
            &input(
                0,
                1.0,
                start + Duration::days(30),
                start + Duration::days(30),
            ),
        );
        assert!(fresh_seed > old_outlink);

        // ...but eventually the waiting outlink wins.
        let much_later = start + Duration::days(60);
        let starved = compute(&w, &input(3, 0.0, start, much_later));
        assert!(starved > fresh_seed, "age boost must eventually win");
    }

    #[test]
    fn out_of_range_signals_are_clamped_not_dropped() {
        let w = PriorityWeights::default();
        let now = datetime!(2026-02-18 12:00 UTC);
        let clamped = compute(
            &w,
            &PriorityInputs {
                depth: 0,
                hint: 5.0,
                staleness: -1.0,
                freshness: 3.0,
                enqueued_at: now,
                now,
            },
        );
        let at_max = compute(
            &w,
            &PriorityInputs {
                depth: 0,
                hint: 1.0,
                staleness: 0.0,
                freshness: 1.0,
                enqueued_at: now,
                now,
            },
        );
        assert_eq!(clamped, at_max);
    }
}
