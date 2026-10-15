//! Per-host politeness: concurrency, inter-request delay, and cooldown.
//!
//! The frontier will not offer a URL for a host that is already busy, still inside its
//! minimum delay since the last request, or in a cooldown imposed because the host asked
//! us to slow down. The delay floor is ours; robots `Crawl-delay` can only raise it.
//!
//! Behaviour adapts to how a host responds:
//!
//! - a `429`/`503` honours the server's `Retry-After`, sets a cooldown, and doubles the
//!   adaptive delay;
//! - other failures raise the delay by a step;
//! - a run of healthy responses lowers it again, slowly.
//!
//! All time is passed in as an `OffsetDateTime`, so the scheduler is deterministic under
//! test and does not read the wall clock itself.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::backoff::ErrorClass;

/// Politeness parameters. Mirrors the `[crawler.politeness]` config block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolitenessConfig {
    /// Maximum requests in flight to one host at once.
    pub max_concurrent_per_host: u32,
    /// Our floor between requests to one host, in milliseconds.
    pub min_inter_request_ms: u64,
    /// Cooldown when a rate limit arrives without a `Retry-After`, in seconds.
    pub fallback_cooldown_secs: u64,
    /// Ceiling on the adaptive component, in milliseconds.
    pub max_adaptive_delay_ms: u64,
    /// Step added to the adaptive delay per adverse event, in milliseconds.
    pub adaptive_step_ms: u64,
    /// Consecutive successes before the adaptive delay is eased.
    pub success_streak_to_relax: u32,
}

impl Default for PolitenessConfig {
    fn default() -> Self {
        Self {
            max_concurrent_per_host: 2,
            min_inter_request_ms: 1000,
            fallback_cooldown_secs: 60,
            max_adaptive_delay_ms: 60_000,
            adaptive_step_ms: 2000,
            success_streak_to_relax: 10,
        }
    }
}

/// The result of asking to fetch a URL for a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Go ahead; nothing else is in flight for this host and the delay has elapsed.
    Admitted,
    /// Too many requests already in flight for this host.
    Busy,
    /// The minimum inter-request delay has not elapsed.
    TooSoon {
        /// When the next request may start.
        until: OffsetDateTime,
    },
    /// A server-imposed cooldown is in effect.
    CoolingDown {
        /// When the cooldown lifts.
        until: OffsetDateTime,
    },
}

/// How a settled request turned out, for adaptation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    /// A usable response.
    Success,
    /// A classified failure.
    Failure(ErrorClass),
}

/// A point-in-time view of one host's politeness, for operator surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPoliteness {
    /// The host.
    pub host: String,
    /// Requests currently in flight.
    pub in_flight: u32,
    /// The full delay applied between requests, in milliseconds.
    pub effective_delay_ms: u64,
    /// When the next request may start.
    pub next_allowed: OffsetDateTime,
    /// When a server-imposed cooldown lifts, if any.
    pub cooldown_until: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
struct HostState {
    in_flight: u32,
    next_allowed: OffsetDateTime,
    cooldown_until: Option<OffsetDateTime>,
    robots_delay_ms: u64,
    adaptive_delay_ms: u64,
    success_streak: u32,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            in_flight: 0,
            next_allowed: OffsetDateTime::UNIX_EPOCH,
            cooldown_until: None,
            robots_delay_ms: 0,
            adaptive_delay_ms: 0,
            success_streak: 0,
        }
    }
}

impl HostState {
    fn base_delay_ms(&self, config: &PolitenessConfig) -> u64 {
        self.robots_delay_ms
            .max(config.min_inter_request_ms)
            .saturating_add(self.adaptive_delay_ms)
    }
}

/// The per-host politeness scheduler.
#[derive(Debug)]
pub struct Politeness {
    config: PolitenessConfig,
    hosts: RwLock<HashMap<String, HostState>>,
}

impl Politeness {
    /// A scheduler with the given config.
    #[must_use]
    pub fn new(config: PolitenessConfig) -> Self {
        Self {
            config,
            hosts: RwLock::new(HashMap::new()),
        }
    }

    /// The active config.
    #[must_use]
    pub fn config(&self) -> &PolitenessConfig {
        &self.config
    }

    /// Record the site's declared `Crawl-delay` for a host, in milliseconds.
    pub fn set_robots_delay_ms(&self, host: &str, millis: u64) {
        if let Ok(mut map) = self.hosts.write() {
            map.entry(host.to_string()).or_default().robots_delay_ms = millis;
        }
    }

    /// Ask to fetch `host` at `now`.
    pub fn try_admit(&self, host: &str, now: OffsetDateTime) -> Admit {
        let Ok(mut map) = self.hosts.write() else {
            return Admit::Busy;
        };
        let state = map.entry(host.to_string()).or_default();

        if let Some(until) = state.cooldown_until {
            if now < until {
                return Admit::CoolingDown { until };
            }
            state.cooldown_until = None;
        }
        if state.in_flight >= self.config.max_concurrent_per_host {
            return Admit::Busy;
        }
        if now < state.next_allowed {
            return Admit::TooSoon {
                until: state.next_allowed,
            };
        }

        state.in_flight += 1;
        Admit::Admitted
    }

    /// Settle a previously admitted request, adapting this host's delay.
    ///
    /// `retry_after_secs` is the server's `Retry-After` when the failure carried one.
    pub fn settle(
        &self,
        host: &str,
        fetched: Fetched,
        retry_after_secs: Option<u64>,
        now: OffsetDateTime,
    ) {
        let Ok(mut map) = self.hosts.write() else {
            return;
        };
        let state = map.entry(host.to_string()).or_default();
        state.in_flight = state.in_flight.saturating_sub(1);

        let mut cooldown_until = None;
        match fetched {
            Fetched::Success => {
                state.success_streak = state.success_streak.saturating_add(1);
                if state.success_streak >= self.config.success_streak_to_relax {
                    state.success_streak = 0;
                    state.adaptive_delay_ms = state
                        .adaptive_delay_ms
                        .saturating_sub(self.config.adaptive_step_ms);
                }
            }
            Fetched::Failure(class) => {
                state.success_streak = 0;
                match class {
                    ErrorClass::Upstream429 => {
                        let secs = retry_after_secs.unwrap_or(self.config.fallback_cooldown_secs);
                        cooldown_until = Some(now + seconds(secs));
                        state.adaptive_delay_ms = self.raised(state.adaptive_delay_ms);
                    }
                    ErrorClass::Upstream5xx => {
                        state.adaptive_delay_ms = self.raised(state.adaptive_delay_ms);
                        if let Some(secs) = retry_after_secs {
                            cooldown_until = Some(now + seconds(secs));
                        }
                    }
                    ErrorClass::Http4xx => {
                        // A 404 storm is wasted capacity; ease off by half a step.
                        state.adaptive_delay_ms = state
                            .adaptive_delay_ms
                            .saturating_add(self.config.adaptive_step_ms / 2)
                            .min(self.config.max_adaptive_delay_ms);
                    }
                    ErrorClass::TimeoutConnect
                    | ErrorClass::DnsFailure
                    | ErrorClass::TlsFailure => {
                        state.adaptive_delay_ms = self.raised(state.adaptive_delay_ms);
                    }
                    // These are our decisions, not the host's behaviour: no adaptation.
                    _ => {}
                }
            }
        }

        let mut next = now + millis(state.base_delay_ms(&self.config));
        if let Some(until) = cooldown_until {
            next = next.max(until);
        }
        state.next_allowed = next;
        state.cooldown_until = cooldown_until;
    }

    /// The full delay currently applied to `host`, in milliseconds.
    #[must_use]
    pub fn effective_delay_ms(&self, host: &str) -> u64 {
        self.hosts
            .read()
            .ok()
            .and_then(|map| map.get(host).map(|s| s.base_delay_ms(&self.config)))
            .unwrap_or(self.config.min_inter_request_ms)
    }

    /// Requests currently in flight for `host`.
    #[must_use]
    pub fn in_flight(&self, host: &str) -> u32 {
        self.hosts
            .read()
            .ok()
            .and_then(|map| map.get(host).map(|s| s.in_flight))
            .unwrap_or(0)
    }

    /// A snapshot of every host seen, for operator surfaces.
    #[must_use]
    pub fn snapshot(&self) -> Vec<HostPoliteness> {
        let Ok(map) = self.hosts.read() else {
            return Vec::new();
        };
        let mut hosts: Vec<HostPoliteness> = map
            .iter()
            .map(|(host, state)| HostPoliteness {
                host: host.clone(),
                in_flight: state.in_flight,
                effective_delay_ms: state.base_delay_ms(&self.config),
                next_allowed: state.next_allowed,
                cooldown_until: state.cooldown_until,
            })
            .collect();
        hosts.sort_by(|a, b| a.host.cmp(&b.host));
        hosts
    }

    fn raised(&self, current: u64) -> u64 {
        current
            .saturating_mul(2)
            .saturating_add(self.config.adaptive_step_ms)
            .min(self.config.max_adaptive_delay_ms)
    }
}

fn seconds(n: u64) -> Duration {
    Duration::seconds(i64::try_from(n).unwrap_or(i64::MAX))
}

fn millis(n: u64) -> Duration {
    Duration::milliseconds(i64::try_from(n).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use time::macros::datetime;

    fn pol(config: PolitenessConfig) -> Politeness {
        Politeness::new(config)
    }

    fn t0() -> OffsetDateTime {
        datetime!(2026-03-01 00:00:00 UTC)
    }

    #[test]
    fn enforces_the_robots_floor() {
        let config = PolitenessConfig {
            max_concurrent_per_host: 5,
            min_inter_request_ms: 2000,
            ..PolitenessConfig::default()
        };
        let p = pol(config);
        assert_eq!(p.effective_delay_ms("a.test"), 2000);
        p.set_robots_delay_ms("a.test", 5000);
        assert_eq!(
            p.effective_delay_ms("a.test"),
            5000,
            "robots raises the floor"
        );
        p.set_robots_delay_ms("a.test", 500);
        assert_eq!(
            p.effective_delay_ms("a.test"),
            2000,
            "robots cannot lower it"
        );
    }

    #[test]
    fn limits_per_host_concurrency() {
        let config = PolitenessConfig {
            max_concurrent_per_host: 1,
            ..PolitenessConfig::default()
        };
        let p = pol(config);
        assert_eq!(p.try_admit("a.test", t0()), Admit::Admitted);
        assert_eq!(p.try_admit("a.test", t0()), Admit::Busy);
        assert_eq!(
            p.try_admit("b.test", t0()),
            Admit::Admitted,
            "other hosts unaffected"
        );
        p.settle("a.test", Fetched::Success, None, t0());
        assert_eq!(p.in_flight("a.test"), 0);
        assert_eq!(
            p.try_admit("a.test", t0() + Duration::seconds(10)),
            Admit::Admitted
        );
    }

    #[test]
    fn enforces_the_inter_request_delay() {
        let config = PolitenessConfig {
            max_concurrent_per_host: 5,
            min_inter_request_ms: 1000,
            ..PolitenessConfig::default()
        };
        let p = pol(config);
        assert_eq!(p.try_admit("a.test", t0()), Admit::Admitted);
        p.settle("a.test", Fetched::Success, None, t0());

        let soon = p.try_admit("a.test", t0() + Duration::milliseconds(500));
        assert_eq!(
            soon,
            Admit::TooSoon {
                until: t0() + Duration::seconds(1)
            }
        );
        assert_eq!(
            p.try_admit("a.test", t0() + Duration::seconds(1)),
            Admit::Admitted
        );
    }

    #[test]
    fn a_rate_limit_sets_a_cooldown_from_retry_after() {
        let p = pol(PolitenessConfig::default());
        p.try_admit("a.test", t0());
        p.settle(
            "a.test",
            Fetched::Failure(ErrorClass::Upstream429),
            Some(120),
            t0(),
        );
        assert_eq!(
            p.try_admit("a.test", t0() + Duration::seconds(60)),
            Admit::CoolingDown {
                until: t0() + Duration::seconds(120)
            }
        );
        assert_eq!(
            p.try_admit("a.test", t0() + Duration::seconds(121)),
            Admit::Admitted
        );
    }

    #[test]
    fn a_rate_limit_without_retry_after_uses_the_fallback() {
        let config = PolitenessConfig {
            fallback_cooldown_secs: 30,
            ..PolitenessConfig::default()
        };
        let p = pol(config);
        p.try_admit("a.test", t0());
        p.settle(
            "a.test",
            Fetched::Failure(ErrorClass::Upstream429),
            None,
            t0(),
        );
        assert_eq!(
            p.try_admit("a.test", t0() + Duration::seconds(10)),
            Admit::CoolingDown {
                until: t0() + Duration::seconds(30)
            }
        );
    }

    #[test]
    fn adaptive_delay_rises_on_failure_and_falls_slowly() {
        let config = PolitenessConfig {
            min_inter_request_ms: 1000,
            max_adaptive_delay_ms: 60_000,
            adaptive_step_ms: 2000,
            success_streak_to_relax: 3,
            ..PolitenessConfig::default()
        };
        let p = pol(config);

        p.try_admit("a.test", t0());
        p.settle(
            "a.test",
            Fetched::Failure(ErrorClass::Upstream5xx),
            None,
            t0(),
        );
        assert_eq!(
            p.effective_delay_ms("a.test"),
            1000 + 2000,
            "doubled (from zero) + step"
        );

        p.try_admit("a.test", t0() + Duration::seconds(60));
        p.settle(
            "a.test",
            Fetched::Failure(ErrorClass::Upstream5xx),
            None,
            t0() + Duration::seconds(60),
        );
        assert_eq!(p.effective_delay_ms("a.test"), 1000 + 6000);

        // A run of successes eases it by one step.
        let mut now = t0() + Duration::seconds(120);
        for _ in 0..3 {
            p.try_admit("a.test", now);
            p.settle("a.test", Fetched::Success, None, now);
            now += Duration::seconds(60);
        }
        assert_eq!(p.effective_delay_ms("a.test"), 1000 + 4000);
    }

    #[test]
    fn adaptive_delay_is_capped() {
        let config = PolitenessConfig {
            max_adaptive_delay_ms: 5000,
            adaptive_step_ms: 10_000,
            ..PolitenessConfig::default()
        };
        let p = pol(config);
        p.try_admit("a.test", t0());
        p.settle(
            "a.test",
            Fetched::Failure(ErrorClass::Upstream5xx),
            None,
            t0(),
        );
        assert_eq!(p.effective_delay_ms("a.test"), 1000 + 5000);
    }

    #[test]
    fn snapshot_lists_hosts_in_order() {
        let p = pol(PolitenessConfig::default());
        p.try_admit("b.test", t0());
        p.try_admit("a.test", t0());
        let snap = p.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].host, "a.test");
        assert_eq!(snap[0].in_flight, 1);
        assert_eq!(snap[1].host, "b.test");
    }
}
