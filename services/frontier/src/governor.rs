//! The crawl governor: one gate that every fetch passes through.
//!
//! Traps, budgets, and per-host politeness are separate rules, but a worker should not
//! have to apply them in the right order or remember to release a slot on every error
//! path. [`Governor::admit`] evaluates all three and returns a single [`Decision`];
//! [`Governor::settle`] releases the slot and records the outcome. Nothing fetches a URL
//! that the governor did not admit into a [`Ticket`].
//!
//! This is the seam the crawl worker will call. The production budget store is Postgres;
//! the in-memory store and a fixed clock make the whole gate deterministic in tests.

use std::sync::Arc;

use lynx_common::time::{day_key, Clock};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;

use crate::budget::{self, BudgetConfig, BudgetError, BudgetTracker, Remaining, Usage};
use crate::politeness::{Admit, Fetched, HostPoliteness, Politeness, PolitenessConfig};
use crate::traps::{self, TrapConfig, TrapKind, TrapVerdict};

/// All governor thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GovernorConfig {
    /// Per-host politeness.
    pub politeness: PolitenessConfig,
    /// Daily budgets.
    pub budget: BudgetConfig,
    /// Trap detection thresholds.
    pub traps: TrapConfig,
}

/// Why a URL was dropped rather than fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// The URL looks like a crawler trap.
    Trap(TrapKind),
    /// A daily budget is exhausted.
    Budget(BudgetError),
}

/// The governor's answer to an admission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Fetch it; settle the [`Ticket`] when done.
    Proceed(Ticket),
    /// Another request to this host is in flight; try a different host.
    Busy,
    /// Wait until `until` (an inter-request delay or a cooldown) before retrying.
    Wait {
        /// When the host may be retried.
        until: OffsetDateTime,
    },
    /// Drop the URL with a recorded reason; do not retry against this budget.
    Drop {
        /// Why.
        reason: DropReason,
    },
}

/// Permission to fetch one URL. Carries the budget bucket it was admitted under so the
/// settle call cannot count it against the wrong day or host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    host: String,
    day: String,
}

impl Ticket {
    /// The host this ticket is for.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// A point-in-time view of the governor, for operator surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernorSnapshot {
    /// The UTC day the budget counters are for.
    pub day: String,
    /// Global usage so far today.
    pub global_usage: Usage,
    /// Per-host politeness.
    pub hosts: Vec<HostPoliteness>,
}

/// The gate every fetch passes through.
#[derive(Debug)]
pub struct Governor<B: BudgetTracker> {
    config: GovernorConfig,
    politeness: Politeness,
    budget: B,
    clock: Arc<dyn Clock>,
}

impl<B: BudgetTracker> Governor<B> {
    /// A governor with the given config, budget store, and clock.
    #[must_use]
    pub fn new(config: GovernorConfig, budget: B, clock: Arc<dyn Clock>) -> Self {
        Self {
            politeness: Politeness::new(config.politeness),
            config,
            budget,
            clock,
        }
    }

    /// Evaluate a fetch of `url` (served by `host`).
    pub fn admit(&self, host: &str, url: &Url) -> Decision {
        if let TrapVerdict::Trap(kind) = traps::detect_with(url, &self.config.traps) {
            return Decision::Drop {
                reason: DropReason::Trap(kind),
            };
        }

        let now = self.clock.now();
        let day = day_key(now);
        let host_usage = self.budget.host_usage(host, &day);
        let global_usage = self.budget.global_usage(&day);
        if let Err(err) = budget::check(&self.config.budget, host_usage, global_usage) {
            return Decision::Drop {
                reason: DropReason::Budget(err),
            };
        }

        match self.politeness.try_admit(host, now) {
            Admit::Admitted => Decision::Proceed(Ticket {
                host: host.to_string(),
                day,
            }),
            Admit::Busy => Decision::Busy,
            Admit::TooSoon { until } | Admit::CoolingDown { until } => Decision::Wait { until },
        }
    }

    /// Settle a fetch admitted by [`admit`](Self::admit): release the slot, adapt the
    /// host's delay, and charge the page and its bytes to today's budget.
    ///
    /// The page is charged whenever a request was attempted; `bytes` is what actually
    /// arrived, or `0` when the fetch failed before a body was read.
    pub fn settle(
        &self,
        ticket: Ticket,
        fetched: Fetched,
        retry_after_secs: Option<u64>,
        bytes: u64,
    ) {
        let Ticket { host, day } = ticket;
        self.politeness
            .settle(&host, fetched, retry_after_secs, self.clock.now());
        self.budget.charge_page(&host, &day);
        if bytes > 0 {
            self.budget.charge_bytes(&host, &day, bytes);
        }
    }

    /// Record a site's `Crawl-delay` for a host, in milliseconds.
    pub fn set_robots_delay_ms(&self, host: &str, millis: u64) {
        self.politeness.set_robots_delay_ms(host, millis);
    }

    /// The budget store.
    #[must_use]
    pub fn budget(&self) -> &B {
        &self.budget
    }

    /// The per-host politeness scheduler.
    #[must_use]
    pub fn politeness(&self) -> &Politeness {
        &self.politeness
    }

    /// The active config.
    #[must_use]
    pub fn config(&self) -> &GovernorConfig {
        &self.config
    }

    /// Budget remaining for `host` today, taking the tighter of the host and global
    /// ceilings.
    #[must_use]
    pub fn remaining(&self, host: &str) -> Remaining {
        let day = day_key(self.clock.now());
        let host_usage = self.budget.host_usage(host, &day);
        let global_usage = self.budget.global_usage(&day);
        Remaining {
            pages: budget::remaining(self.config.budget.max_pages_per_host_day, host_usage.pages)
                .min(budget::remaining(
                    self.config.budget.max_pages_global_day,
                    global_usage.pages,
                )),
            bytes: budget::remaining(self.config.budget.max_bytes_per_host_day, host_usage.bytes)
                .min(budget::remaining(
                    self.config.budget.max_bytes_global_day,
                    global_usage.bytes,
                )),
        }
    }

    /// A snapshot for operator surfaces.
    #[must_use]
    pub fn snapshot(&self) -> GovernorSnapshot {
        let now = self.clock.now();
        let day = day_key(now);
        GovernorSnapshot {
            global_usage: self.budget.global_usage(&day),
            hosts: self.politeness.snapshot(),
            day,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::InMemoryBudgetTracker;
    use lynx_common::time::FixedClock;
    use pretty_assertions::assert_eq;
    use time::macros::datetime;
    use time::Duration;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    fn clock() -> (Arc<FixedClock>, Arc<dyn Clock>) {
        let fixed = Arc::new(FixedClock::at(datetime!(2026-03-01 00:00 UTC)));
        let dynamic: Arc<dyn Clock> = fixed.clone();
        (fixed, dynamic)
    }

    fn governor(config: GovernorConfig) -> (Arc<FixedClock>, Governor<InMemoryBudgetTracker>) {
        let (clock, dynamic) = clock();
        let g = Governor::new(config, InMemoryBudgetTracker::new(), dynamic);
        (clock, g)
    }

    #[test]
    fn admits_an_ordinary_url_and_charges_it() {
        let (_, g) = governor(GovernorConfig::default());
        let Decision::Proceed(ticket) = g.admit("e.com", &url("https://e.com/a")) else {
            panic!("expected Proceed");
        };
        assert_eq!(ticket.host(), "e.com");
        g.settle(ticket, Fetched::Success, None, 500);

        let usage = g.budget().host_usage("e.com", "2026-03-01");
        assert_eq!(
            usage,
            Usage {
                pages: 1,
                bytes: 500
            }
        );
        assert_eq!(
            g.budget().global_usage("2026-03-01"),
            Usage {
                pages: 1,
                bytes: 500
            }
        );
    }

    #[test]
    fn drops_a_trap_before_touching_the_budget() {
        let (_, g) = governor(GovernorConfig::default());
        let long = "a".repeat(5000);
        let decision = g.admit("e.com", &url(&format!("https://e.com/{long}")));
        assert_eq!(
            decision,
            Decision::Drop {
                reason: DropReason::Trap(TrapKind::LongUrl)
            }
        );
    }

    #[test]
    fn drops_when_the_host_budget_is_exhausted() {
        let (_, g) = governor(GovernorConfig::default());
        g.budget().set_host_usage(
            "e.com",
            "2026-03-01",
            Usage {
                pages: 5000,
                bytes: 0,
            },
        );
        assert_eq!(
            g.admit("e.com", &url("https://e.com/a")),
            Decision::Drop {
                reason: DropReason::Budget(BudgetError::HostPages)
            }
        );
    }

    #[test]
    fn drops_when_the_global_budget_is_exhausted() {
        let (_, g) = governor(GovernorConfig::default());
        g.budget().set_global_usage(
            "2026-03-01",
            Usage {
                pages: 2_000_000,
                bytes: 0,
            },
        );
        assert_eq!(
            g.admit("e.com", &url("https://e.com/a")),
            Decision::Drop {
                reason: DropReason::Budget(BudgetError::GlobalPages)
            }
        );
    }

    #[test]
    fn reports_busy_and_wait() {
        let config = GovernorConfig {
            politeness: PolitenessConfig {
                max_concurrent_per_host: 1,
                ..PolitenessConfig::default()
            },
            ..GovernorConfig::default()
        };
        let (clock, g) = governor(config);

        let Decision::Proceed(ticket) = g.admit("e.com", &url("https://e.com/a")) else {
            panic!("expected Proceed");
        };
        assert_eq!(g.admit("e.com", &url("https://e.com/b")), Decision::Busy);

        // Settle; now the host is delayed, not busy.
        g.settle(ticket, Fetched::Success, None, 10);
        let wait = g.admit("e.com", &url("https://e.com/b"));
        let Decision::Wait { until } = wait else {
            panic!("expected Wait, got {wait:?}");
        };
        assert_eq!(until, datetime!(2026-03-01 00:00:01 UTC));

        clock.advance(Duration::seconds(2));
        assert!(matches!(
            g.admit("e.com", &url("https://e.com/b")),
            Decision::Proceed(_)
        ));
    }

    #[test]
    fn a_rate_limit_puts_the_host_in_cooldown() {
        let (clock, g) = governor(GovernorConfig::default());
        let Decision::Proceed(ticket) = g.admit("e.com", &url("https://e.com/a")) else {
            panic!("expected Proceed");
        };
        g.settle(
            ticket,
            Fetched::Failure(crate::backoff::ErrorClass::Upstream429),
            Some(300),
            0,
        );
        assert_eq!(
            g.admit("e.com", &url("https://e.com/b")),
            Decision::Wait {
                until: datetime!(2026-03-01 00:05:00 UTC)
            }
        );
        clock.advance(Duration::seconds(301));
        assert!(matches!(
            g.admit("e.com", &url("https://e.com/b")),
            Decision::Proceed(_)
        ));
    }

    #[test]
    fn remaining_takes_the_tighter_of_host_and_global() {
        let (_, g) = governor(GovernorConfig::default());
        // A fresh host against default ceilings.
        let fresh = g.remaining("e.com");
        assert_eq!(fresh.pages, 5000);
        assert_eq!(fresh.bytes, 5 * crate::budget::GIB);

        // The global ceiling is tighter than the host's once it is nearly spent.
        g.budget().set_global_usage(
            "2026-03-01",
            Usage {
                pages: 1_999_000,
                bytes: 0,
            },
        );
        assert_eq!(g.remaining("e.com").pages, 1000);
    }

    #[test]
    fn snapshot_reports_the_day_and_global_usage() {
        let (_, g) = governor(GovernorConfig::default());
        let Decision::Proceed(ticket) = g.admit("e.com", &url("https://e.com/a")) else {
            panic!("expected Proceed");
        };
        g.settle(ticket, Fetched::Success, None, 42);
        let snap = g.snapshot();
        assert_eq!(snap.day, "2026-03-01");
        assert_eq!(
            snap.global_usage,
            Usage {
                pages: 1,
                bytes: 42
            }
        );
        assert_eq!(snap.hosts.len(), 1);
        assert_eq!(snap.hosts[0].host, "e.com");
    }
}
