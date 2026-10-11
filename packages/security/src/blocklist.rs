//! Host and domain blocklists for crawl safety and policy compliance.
//!
//! Matching is on canonical, lowercased host names only. An exact-host rule blocks just
//! that host; a domain rule blocks the registrable domain and all subdomains. This is
//! the mechanism behind a takedown or a legal block — it must be auditable, so every
//! entry carries a reason that is safe to show in an operator UI.

use std::collections::HashSet;

/// A blocklist entry with its justification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRule {
    /// The exact host, or a bare registrable domain (no leading dot).
    pub pattern: String,
    /// If true, `pattern` is a domain and matches all subdomains.
    pub is_domain: bool,
    /// Human-readable reason, safe to display. Never contains user data.
    pub reason: String,
}

impl BlockRule {
    /// A rule for one exact host.
    #[must_use]
    pub fn host(host: &str, reason: impl Into<String>) -> Self {
        Self {
            pattern: host.to_ascii_lowercase(),
            is_domain: false,
            reason: reason.into(),
        }
    }

    /// A rule for a domain and all its subdomains.
    #[must_use]
    pub fn domain(domain: &str, reason: impl Into<String>) -> Self {
        Self {
            pattern: domain.to_ascii_lowercase(),
            is_domain: true,
            reason: reason.into(),
        }
    }

    /// Whether this rule matches `host`.
    #[must_use]
    pub fn matches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        if self.is_domain {
            host == self.pattern
                || host
                    .strip_suffix(&self.pattern)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        } else {
            host == self.pattern
        }
    }
}

/// A set of block rules with an exact-match fast path.
#[derive(Debug, Default)]
pub struct HostBlocklist {
    exact: HashSet<String>,
    rules: Vec<BlockRule>,
}

impl HostBlocklist {
    /// An empty blocklist.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from rules.
    #[must_use]
    pub fn from_rules(rules: Vec<BlockRule>) -> Self {
        let mut list = Self::new();
        for rule in rules {
            list.add(rule);
        }
        list
    }

    /// Add a rule.
    pub fn add(&mut self, rule: BlockRule) {
        if !rule.is_domain {
            self.exact.insert(rule.pattern.clone());
        }
        self.rules.push(rule);
    }

    /// The first rule that blocks `host`, if any.
    #[must_use]
    pub fn blocking_reason(&self, host: &str) -> Option<&str> {
        let lowered = host.to_ascii_lowercase();
        if self.exact.contains(&lowered) {
            return self
                .rules
                .iter()
                .find(|r| !r.is_domain && r.pattern == lowered)
                .map(|r| r.reason.as_str());
        }
        self.rules
            .iter()
            .find(|r| r.is_domain && r.matches(host))
            .map(|r| r.reason.as_str())
    }

    /// Whether `host` is blocked.
    #[must_use]
    pub fn is_blocked(&self, host: &str) -> bool {
        self.blocking_reason(host).is_some()
    }

    /// Number of rules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether there are no rules.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> HostBlocklist {
        HostBlocklist::from_rules(vec![
            BlockRule::host("bad.example", "manual ban"),
            BlockRule::domain("spam.test", "spam network"),
        ])
    }

    #[test]
    fn exact_host_matches_only_that_host() {
        let l = list();
        assert!(l.is_blocked("bad.example"));
        assert!(l.is_blocked("BAD.EXAMPLE"));
        assert!(!l.is_blocked("sub.bad.example"));
    }

    #[test]
    fn domain_matches_subdomains_and_apex() {
        let l = list();
        assert!(l.is_blocked("spam.test"));
        assert!(l.is_blocked("a.b.spam.test"));
        assert!(!l.is_blocked("notspam.test"));
        assert!(!l.is_blocked("spam.test.evil.com"));
    }

    #[test]
    fn reports_reason() {
        let l = list();
        assert_eq!(l.blocking_reason("a.spam.test"), Some("spam network"));
        assert_eq!(l.blocking_reason("clean.example"), None);
    }
}
