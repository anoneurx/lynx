# ADR 0014: A/B Testing Framework

**Date:** 2024-05-01
**Status:** Accepted
**Deciders:** Product, Search Team, Data Team
**Tags:** ab-testing, experimentation

## Context

Need to safely test:
- Ranking model changes (weights, features, algorithms)
- UI changes (snippet length, highlighting)
- Crawl/index changes (freshness, coverage)
- Configuration changes (timeouts, cache TTL)

Requirements:
- Random assignment, consistent per user
- Guardrails (error rate, latency, zero-result rate)
- Statistical rigor (sequential testing)
- Minimal code changes per experiment

## Decision

**Cookie-based assignment + feature flags + sequential analysis.**

### Assignment

```rust
fn assign_experiment(req: &Request, exp_id: &str) -> Group {
    // 1. Check existing cookie
    if let Some(cookie) = req.cookie(&format!("lynx_exp_{exp_id}")) {
        return cookie.value().parse().unwrap();
    }

    // 2. Deterministic hash (user_id or IP + UA)
    let key = req.user_id()
        .or_else(|| req.client_ip())
        .unwrap_or_else(|| req.header("user-agent").unwrap_or("anon"));
    let hash = blake3_hash(format!("{exp_id}:{key}"));
    let group = if hash[0] < 128 { Group::Treatment } else { Group::Control };

    // 3. Set cookie (1 year, HttpOnly, Secure, SameSite=Lax)
    group
}
```

### Feature Flags (LaunchDarkly-style, in-memory)

```toml
# config/experiments.toml
[exp-2026-09-ltr]
enabled = true
traffic_split = 0.5
groups = ["control", "treatment"]
guardrails = { error_rate = 0.01, latency_p99_ms = 300, zero_result_rate = 0.05 }
```

### Sequential Testing (Always-Valid p-values)

Use **mSPRT** (mixture Sequential Probability Ratio Test):

```python
# No peeking problem; stop anytime
from scipy.stats import norm

def msprt(control, treatment, alpha=0.05, beta=0.2, min_effect=0.01):
    # Returns (should_stop, reject_null, p_value)
    ...
```

### Guardrails (Auto-Stop)

| Metric | Threshold | Action |
| ------ | --------- | ------ |
| Error rate (5xx) | > 1% | Pause experiment |
| Latency p99 | > 300ms | Pause experiment |
| Zero-result rate | > 5% | Pause experiment |
| Revenue proxy | < -2% | Pause experiment |

## Consequences

### Positive
- **Safe**: Guardrails prevent bad launches
- **Fast**: Sequential testing → stop early if significant
- **Flexible**: Any config change can be experiment
- **Auditable**: Assignment logged (hash only), decisions recorded

### Negative
- **Cookie dependence** → fails for cookie-less clients (bots, curl)
- **Mitigation**: Fallback to IP+UA hash; bots get control

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| User ID only | Not all users logged in |
| Request-level random | Inconsistent experience; no session continuity |
| Third-party (Optimizely) | Cost, latency, vendor lock-in |

## Related

- ADR 0013: Ranking Pipeline (model experiments)
- ADR 0008: No Query Logging (experiment logs)
- ADR 0019: Telemetry (experiment metrics)