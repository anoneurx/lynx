//! Daily crawl budgets, per host and global.
//!
//! Politeness (rate) and budget (volume) are different controls. A host can be perfectly
//! fast and still be crawled too much; a budget is the ceiling that says "no more from
//! this host today", independent of robots and independent of how idle we are.
//!
//! Budgets are counted per UTC day so they reset without a scheduler. In production the
//! counters live in Postgres (`crawl_budget`) so they are shared across workers; this
//! module defines the trait and an in-memory implementation that mirrors the same shape
//! for tests and single-process runs.
//!
//! Exhaustion is a *policy decision*: the URL is dropped with a recorded reason, not
//! retried until the budget resets.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// The byte budget constants, in bytes.
pub const GIB: u64 = 1024 * 1024 * 1024;

/// Budget ceilings. Mirrors the `[crawler.budget]` config block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Maximum pages fetched from one host per UTC day.
    pub max_pages_per_host_day: u64,
    /// Maximum bytes fetched from one host per UTC day.
    pub max_bytes_per_host_day: u64,
    /// Maximum pages fetched across all hosts per UTC day.
    pub max_pages_global_day: u64,
    /// Maximum bytes fetched across all hosts per UTC day.
    pub max_bytes_global_day: u64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_pages_per_host_day: 5000,
            max_bytes_per_host_day: 5 * GIB,
            max_pages_global_day: 2_000_000,
            max_bytes_global_day: 200 * GIB,
        }
    }
}

/// Pages and bytes counted against a budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Pages fetched.
    pub pages: u64,
    /// Bytes fetched.
    pub bytes: u64,
}

/// How much budget is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remaining {
    /// Pages left in `[0, max]`.
    pub pages: u64,
    /// Bytes left in `[0, max]`.
    pub bytes: u64,
}

impl Remaining {
    /// Whether any budget remains.
    #[must_use]
    pub fn is_exhausted(self) -> bool {
        self.pages == 0 || self.bytes == 0
    }
}

/// Which budget is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetError {
    /// This host has fetched its daily page allowance.
    HostPages,
    /// This host has fetched its daily byte allowance.
    HostBytes,
    /// The whole crawl has fetched its daily page allowance.
    GlobalPages,
    /// The whole crawl has fetched its daily byte allowance.
    GlobalBytes,
}

impl BudgetError {
    /// A short, stable code for logs and the attempt record.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::HostPages => "budget_host_pages",
            Self::HostBytes => "budget_host_bytes",
            Self::GlobalPages => "budget_global_pages",
            Self::GlobalBytes => "budget_global_bytes",
        }
    }
}

/// Remaining budget for `usage` against `max`.
#[must_use]
pub fn remaining(max: u64, usage: u64) -> u64 {
    max.saturating_sub(usage)
}

/// Whether a fetch may proceed, given the host and global usage.
///
/// Host budgets are checked before the global budget, so the recorded reason is the most
/// specific one.
///
/// # Errors
///
/// Returns the most specific [`BudgetError`] if a host or global budget for the day is
/// already exhausted.
pub fn check(config: &BudgetConfig, host: Usage, global: Usage) -> Result<(), BudgetError> {
    if host.pages >= config.max_pages_per_host_day {
        return Err(BudgetError::HostPages);
    }
    if host.bytes >= config.max_bytes_per_host_day {
        return Err(BudgetError::HostBytes);
    }
    if global.pages >= config.max_pages_global_day {
        return Err(BudgetError::GlobalPages);
    }
    if global.bytes >= config.max_bytes_global_day {
        return Err(BudgetError::GlobalBytes);
    }
    Ok(())
}

/// The counter store the governor reads and charges.
///
/// `day` is a `YYYY-MM-DD` UTC key (see `lynx_common::time::day_key`), passed in rather
/// than read from a clock so the caller controls the bucket.
pub trait BudgetTracker: Send + Sync {
    /// Pages and bytes counted for `host` on `day`.
    fn host_usage(&self, host: &str, day: &str) -> Usage;

    /// Pages and bytes counted across all hosts on `day`.
    fn global_usage(&self, day: &str) -> Usage;

    /// Record one page fetched from `host`.
    fn charge_page(&self, host: &str, day: &str);

    /// Record `bytes` fetched from `host`.
    fn charge_bytes(&self, host: &str, day: &str, bytes: u64);
}

/// An in-memory [`BudgetTracker`]. Not shared across processes; the Postgres store is the
/// production implementation of the same trait.
#[derive(Debug, Default)]
pub struct InMemoryBudgetTracker {
    hosts: RwLock<HashMap<(String, String), Usage>>,
    global: RwLock<HashMap<String, Usage>>,
}

impl InMemoryBudgetTracker {
    /// An empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a host's usage, for tests and restore-from-database.
    pub fn set_host_usage(&self, host: &str, day: &str, usage: Usage) {
        if let Ok(mut map) = self.hosts.write() {
            map.insert((host.to_string(), day.to_string()), usage);
        }
    }

    /// Seed the global usage, for tests and restore-from-database.
    pub fn set_global_usage(&self, day: &str, usage: Usage) {
        if let Ok(mut map) = self.global.write() {
            map.insert(day.to_string(), usage);
        }
    }
}

impl BudgetTracker for InMemoryBudgetTracker {
    fn host_usage(&self, host: &str, day: &str) -> Usage {
        self.hosts
            .read()
            .ok()
            .and_then(|map| map.get(&(host.to_string(), day.to_string())).copied())
            .unwrap_or_default()
    }

    fn global_usage(&self, day: &str) -> Usage {
        self.global
            .read()
            .ok()
            .and_then(|map| map.get(day).copied())
            .unwrap_or_default()
    }

    fn charge_page(&self, host: &str, day: &str) {
        if let Ok(mut map) = self.hosts.write() {
            let entry = map.entry((host.to_string(), day.to_string())).or_default();
            entry.pages += 1;
        }
        if let Ok(mut map) = self.global.write() {
            map.entry(day.to_string()).or_default().pages += 1;
        }
    }

    fn charge_bytes(&self, host: &str, day: &str, bytes: u64) {
        if let Ok(mut map) = self.hosts.write() {
            let entry = map.entry((host.to_string(), day.to_string())).or_default();
            entry.bytes = entry.bytes.saturating_add(bytes);
        }
        if let Ok(mut map) = self.global.write() {
            let entry = map.entry(day.to_string()).or_default();
            entry.bytes = entry.bytes.saturating_add(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn charging_accumulates_per_host_and_globally() {
        let tracker = InMemoryBudgetTracker::new();
        tracker.charge_page("a.test", "2026-03-01");
        tracker.charge_page("a.test", "2026-03-01");
        tracker.charge_page("b.test", "2026-03-01");
        tracker.charge_bytes("a.test", "2026-03-01", 100);
        tracker.charge_bytes("b.test", "2026-03-01", 50);

        assert_eq!(
            tracker.host_usage("a.test", "2026-03-01"),
            Usage {
                pages: 2,
                bytes: 100
            }
        );
        assert_eq!(
            tracker.global_usage("2026-03-01"),
            Usage {
                pages: 3,
                bytes: 150
            }
        );
    }

    #[test]
    fn budgets_reset_on_a_new_day() {
        let tracker = InMemoryBudgetTracker::new();
        tracker.charge_page("a.test", "2026-03-01");
        assert_eq!(tracker.host_usage("a.test", "2026-03-02"), Usage::default());
        assert_eq!(tracker.global_usage("2026-03-02"), Usage::default());
    }

    #[test]
    fn an_unknown_host_has_no_usage() {
        let tracker = InMemoryBudgetTracker::new();
        assert_eq!(
            tracker.host_usage("nobody.test", "2026-03-01"),
            Usage::default()
        );
    }

    #[test]
    fn check_reports_the_most_specific_exhaustion() {
        let config = BudgetConfig {
            max_pages_per_host_day: 10,
            max_bytes_per_host_day: 1000,
            max_pages_global_day: 100,
            max_bytes_global_day: 10_000,
        };
        let ok = check(&config, Usage::default(), Usage::default());
        assert_eq!(ok, Ok(()));

        assert_eq!(
            check(
                &config,
                Usage {
                    pages: 10,
                    bytes: 0
                },
                Usage::default()
            ),
            Err(BudgetError::HostPages)
        );
        assert_eq!(
            check(
                &config,
                Usage {
                    pages: 0,
                    bytes: 1000
                },
                Usage::default()
            ),
            Err(BudgetError::HostBytes)
        );
        assert_eq!(
            check(
                &config,
                Usage::default(),
                Usage {
                    pages: 100,
                    bytes: 0
                }
            ),
            Err(BudgetError::GlobalPages)
        );
        assert_eq!(
            check(
                &config,
                Usage::default(),
                Usage {
                    pages: 0,
                    bytes: 10_000
                }
            ),
            Err(BudgetError::GlobalBytes)
        );
    }

    #[test]
    fn bytes_saturate_rather_than_overflow() {
        let tracker = InMemoryBudgetTracker::new();
        tracker.charge_bytes("a.test", "2026-03-01", u64::MAX);
        tracker.charge_bytes("a.test", "2026-03-01", 10);
        assert_eq!(tracker.host_usage("a.test", "2026-03-01").bytes, u64::MAX);
    }

    #[test]
    fn remaining_saturates_at_zero() {
        assert_eq!(remaining(5, 3), 2);
        assert_eq!(remaining(5, 5), 0);
        assert_eq!(remaining(5, 9), 0);
    }
}
