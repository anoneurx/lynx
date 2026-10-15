//! Classified retry and backoff.
//!
//! "Retry everything three times" is how a crawler turns a transient outage at
//! someone else's host into a self-inflicted denial of service. Retries are
//! therefore classified: a 4xx is permanent, a 429 is mandatory backoff, a 5xx is
//! exponential until the ceiling. Jitter is mandatory — without it every worker
//! retries the same host at the same instant, which is exactly how a crawler gets
//! blocked.
//!
//! The jitter *value* is a parameter, not a hidden call to an RNG, so the policy is
//! deterministic and testable. The caller supplies a value in `[-1.0, 1.0]`.

use serde::{Deserialize, Serialize};

/// A classified fetch failure. The variant names match the Postgres `error_class`
/// enum so a value can round-trip without a mapping table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorClass {
    /// Connection timed out.
    TimeoutConnect,
    /// DNS resolution failed.
    DnsFailure,
    /// Upstream returned 5xx.
    #[serde(rename = "upstream_5xx")]
    Upstream5xx,
    /// Upstream returned 429.
    #[serde(rename = "upstream_429")]
    Upstream429,
    /// Upstream returned 4xx (other than 408/429).
    #[serde(rename = "http_4xx")]
    Http4xx,
    /// Body exceeded the byte cap.
    ContentTooLarge,
    /// Content type is not one we index.
    UnsupportedContentType,
    /// robots.txt disallowed the URL.
    #[serde(rename = "robots_disallow")]
    RobotsDisallow,
    /// The safety gate (SSRF control) denied the URL.
    SafetyDenied,
    /// The host's daily budget is exhausted.
    BudgetExhausted,
    /// Parsing failed.
    ParseFailed,
    /// Indexing failed on our side.
    IndexFailed,
    /// A redirect loop.
    RedirectLoop,
    /// TLS handshake or certificate failure.
    TlsFailure,
}

/// The retry shape for one error class.
#[allow(clippy::struct_excessive_bools)] // a set of independent flags is the clearest shape here
#[derive(Debug, Clone, Copy)]
struct ClassRules {
    retry: bool,
    /// Permanent after the first retry (used for `parse_failed`).
    retry_once: bool,
    /// A server-supplied `Retry-After` is authoritative when present.
    honours_retry_after: bool,
    /// Record the URL as denied (so it is not re-evaluated on every discovery).
    mark_denied: bool,
}

impl ClassRules {
    const fn no_retry() -> Self {
        Self {
            retry: false,
            retry_once: false,
            honours_retry_after: false,
            mark_denied: false,
        }
    }
}

fn classify(class: ErrorClass) -> ClassRules {
    match class {
        ErrorClass::TimeoutConnect
        | ErrorClass::DnsFailure
        | ErrorClass::TlsFailure
        | ErrorClass::IndexFailed => ClassRules {
            retry: true,
            retry_once: false,
            honours_retry_after: false,
            mark_denied: false,
        },
        ErrorClass::Upstream5xx | ErrorClass::Upstream429 => ClassRules {
            retry: true,
            retry_once: false,
            honours_retry_after: true,
            mark_denied: false,
        },
        ErrorClass::ParseFailed => ClassRules {
            retry: true,
            retry_once: true,
            honours_retry_after: false,
            mark_denied: false,
        },
        ErrorClass::RobotsDisallow => ClassRules {
            mark_denied: true,
            ..ClassRules::no_retry()
        },
        ErrorClass::Http4xx
        | ErrorClass::ContentTooLarge
        | ErrorClass::UnsupportedContentType
        | ErrorClass::SafetyDenied
        | ErrorClass::BudgetExhausted
        | ErrorClass::RedirectLoop => ClassRules::no_retry(),
    }
}

/// Backoff parameters. Mirror of the `[crawler.retry]` config block.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BackoffPolicy {
    /// First exponential step, in seconds.
    pub base_seconds: u64,
    /// Ceiling on any computed delay, in seconds.
    pub max_seconds: u64,
    /// Jitter as a fraction of the delay, in `[0,1]`.
    pub jitter_fraction: f64,
    /// Hard attempt ceiling; past it, no class retries.
    pub max_attempts: u16,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base_seconds: 30,
            max_seconds: 6 * 3600,
            jitter_fraction: 0.25,
            max_attempts: 10,
        }
    }
}

/// What to do after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPlan {
    /// Whether the URL should be retried.
    pub retry: bool,
    /// Delay before the next attempt, in seconds.
    pub delay_seconds: Option<u64>,
    /// Whether to record the URL as denied rather than merely dropped.
    pub mark_denied: bool,
}

/// Decide what to do about a failure.
///
/// `attempts` is the number of attempts already made (so a first failure is `1`).
/// `retry_after` is a server instruction in seconds, if any. `jitter` is a factor in
/// `[-1.0, 1.0]`; `0.0` disables jitter for tests.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn plan(
    policy: &BackoffPolicy,
    class: ErrorClass,
    attempts: u16,
    retry_after: Option<u64>,
    jitter: f64,
) -> RetryPlan {
    let rules = classify(class);

    if !rules.retry {
        return RetryPlan {
            retry: false,
            delay_seconds: None,
            mark_denied: rules.mark_denied,
        };
    }

    let exhausted = attempts >= policy.max_attempts || (rules.retry_once && attempts >= 2);
    if exhausted {
        return RetryPlan {
            retry: false,
            delay_seconds: None,
            mark_denied: false,
        };
    }

    // Exponential component: base * 2^(attempts-1), capped.
    let shift = u32::from(attempts.saturating_sub(1)).min(32);
    let exponential = policy
        .base_seconds
        .saturating_mul(1u64 << shift)
        .min(policy.max_seconds);

    // A server Retry-After is authoritative; we do not jitter an instruction.
    let delay = if rules.honours_retry_after {
        retry_after.unwrap_or(exponential)
    } else {
        let factor = 1.0 + policy.jitter_fraction * jitter.clamp(-1.0, 1.0);
        ((exponential as f64) * factor).round().max(0.0) as u64
    };

    RetryPlan {
        retry: true,
        delay_seconds: Some(delay),
        mark_denied: false,
    }
}

/// The scope a ban applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BanScope {
    /// A single URL.
    Url,
    /// A whole host.
    Host,
    /// A registrable domain.
    Domain,
}

/// A ban with a reason implied by the caller and a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ban {
    /// What is banned.
    pub scope: BanScope,
    /// How long, in seconds.
    pub duration_seconds: u64,
}

/// Escalating host ban by consecutive failure count: 15 min → 2 h → 24 h → 7 days.
///
/// Returns `None` below the first threshold, so an occasional failure is treated as
/// exactly that.
#[must_use]
pub fn escalating_host_ban(consecutive_failures: u32) -> Option<Ban> {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let duration_seconds = match consecutive_failures {
        12.. => 7 * DAY,
        9..=11 => DAY,
        6..=8 => 2 * HOUR,
        3..=5 => 15 * MINUTE,
        _ => return None,
    };
    Some(Ban {
        scope: BanScope::Host,
        duration_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn policy() -> BackoffPolicy {
        BackoffPolicy::default()
    }

    #[test]
    fn client_error_is_permanent() {
        let plan = plan(&policy(), ErrorClass::Http4xx, 1, None, 0.0);
        assert!(!plan.retry);
        assert_eq!(plan.delay_seconds, None);
        assert!(!plan.mark_denied);
    }

    #[test]
    fn rate_limit_honours_retry_after_mandatorily() {
        let plan = plan(&policy(), ErrorClass::Upstream429, 1, Some(120), 0.9);
        assert!(plan.retry);
        assert_eq!(plan.delay_seconds, Some(120), "Retry-After is not jittered");
    }

    #[test]
    fn rate_limit_without_retry_after_falls_back_to_exponential() {
        let plan = plan(&policy(), ErrorClass::Upstream429, 2, None, 0.0);
        assert_eq!(plan.delay_seconds, Some(60));
    }

    #[test]
    fn server_error_honours_retry_after_when_present() {
        let honoured = plan(&policy(), ErrorClass::Upstream5xx, 1, Some(90), 0.0);
        assert_eq!(honoured.delay_seconds, Some(90));
        let fallback = plan(&policy(), ErrorClass::Upstream5xx, 1, None, 0.0);
        assert_eq!(fallback.delay_seconds, Some(30));
    }

    #[test]
    fn exponential_growth_doubles_each_attempt() {
        let p = policy();
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 1, None, 0.0).delay_seconds,
            Some(30)
        );
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 2, None, 0.0).delay_seconds,
            Some(60)
        );
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 3, None, 0.0).delay_seconds,
            Some(120)
        );
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 4, None, 0.0).delay_seconds,
            Some(240)
        );
    }

    #[test]
    fn delay_is_capped() {
        // Raise the ceiling so the cap, not the attempt limit, is what binds.
        let p = BackoffPolicy {
            base_seconds: 30,
            max_seconds: 300,
            jitter_fraction: 0.0,
            max_attempts: 20,
        };
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 3, None, 0.0).delay_seconds,
            Some(120)
        );
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 4, None, 0.0).delay_seconds,
            Some(240)
        );
        assert_eq!(
            plan(&p, ErrorClass::TimeoutConnect, 5, None, 0.0).delay_seconds,
            Some(300),
            "capped at max_seconds"
        );
    }

    #[test]
    fn jitter_is_bounded_by_the_fraction() {
        let p = policy();
        let low = plan(&p, ErrorClass::DnsFailure, 1, None, -1.0)
            .delay_seconds
            .unwrap();
        let high = plan(&p, ErrorClass::DnsFailure, 1, None, 1.0)
            .delay_seconds
            .unwrap();
        assert_eq!(low, 23); // 30 * 0.75 = 22.5, rounded half away from zero
        assert_eq!(high, 38); // 30 * 1.25 = 37.5, rounded
    }

    #[test]
    fn parse_failure_retries_once_then_gives_up() {
        let p = policy();
        assert!(plan(&p, ErrorClass::ParseFailed, 1, None, 0.0).retry);
        assert!(!plan(&p, ErrorClass::ParseFailed, 2, None, 0.0).retry);
    }

    #[test]
    fn robots_disallow_is_recorded_as_denied_not_retried() {
        let plan = plan(&policy(), ErrorClass::RobotsDisallow, 1, None, 0.0);
        assert!(!plan.retry);
        assert!(plan.mark_denied);
    }

    #[test]
    fn attempt_ceiling_stops_all_retries() {
        let p = policy();
        let at_ceiling = plan(&p, ErrorClass::IndexFailed, p.max_attempts, None, 0.0);
        assert!(!at_ceiling.retry);
    }

    #[test]
    fn host_bans_escalate() {
        assert_eq!(escalating_host_ban(2), None);
        assert_eq!(escalating_host_ban(3).unwrap().duration_seconds, 15 * 60);
        assert_eq!(escalating_host_ban(6).unwrap().duration_seconds, 2 * 3600);
        assert_eq!(escalating_host_ban(9).unwrap().duration_seconds, 24 * 3600);
        assert_eq!(
            escalating_host_ban(20).unwrap().duration_seconds,
            7 * 24 * 3600
        );
        assert_eq!(escalating_host_ban(5).unwrap().scope, BanScope::Host);
    }
}
