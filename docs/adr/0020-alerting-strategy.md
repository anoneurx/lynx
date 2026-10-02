# ADR 0020: Alerting Strategy

**Date:** 2024-08-01
**Status:** Accepted
**Deciders:** SRE, Platform Team
**Tags:** alerting, prometheus, pagerduty

## Context

Alerting principles:
- **Actionable**: Every alert has a runbook
- **No noise**: Page only for user-impacting issues
- **Tiered**: Critical (page) → Warning (ticket) → Info (dashboard)
- **SLO-based**: Alert on error budget burn, not raw metrics

## Decision

**PrometheusRule + Alertmanager → PagerDuty (critical) / GitHub Issues (warning).**

### Alert Categories

| Severity | Notification | Response Time | Examples |
| -------- | ------------ | ------------- | -------- |
| **Critical** | PagerDuty (page) | 5 min | API 5xx > 5%, crawl queue > 100k, index stale > 24h |
| **Warning** | GitHub Issue (auto) | 1 hr | Latency p99 > 500ms, PG replication lag > 30s, Redis memory > 85% |
| **Info** | Dashboard only | — | New deployment, config change, index generation published |

### Critical Alerts (Page)

```yaml
# Critical: User-facing outage
- alert: LynxAPIHighErrorRate
  expr: |
    sum(rate(http_requests_total{status=~"5.."}[5m])) by (job)
    /
    sum(rate(http_requests_total[5m])) by (job)
    > 0.05
  for: 2m
  labels:
    severity: critical
    runbook: "https://github.com/lynx/lynx/blob/main/docs/security/RB-01.md"
  annotations:
    summary: "API 5xx rate > 5% for 2m"
    description: "{{ $value | humanizePercentage }} of requests failing"

- alert: LynxCrawlerQueueBacklog
  expr: lynx_crawl_queue_depth > 100000
  for: 10m
  labels:
    severity: critical
    runbook: "https://github.com/lynx/lynx/blob/main/docs/security/RB-04.md"
  annotations:
    summary: "Crawl queue backlog > 100k for 10m"

- alert: LynxIndexerGenerationStale
  expr: time() - lynx_indexer_last_generation_timestamp > 86400
  for: 5m
  labels:
    severity: critical
    runbook: "https://github.com/lynx/lynx/blob/main/docs/security/RB-05.md"
  annotations:
    summary: "No new index generation in 24h"
```

### Warning Alerts (Ticket)

```yaml
- alert: LynxAPIHighLatency
  expr: histogram_quantile(0.99, rate(http_request_duration_seconds_bucket[5m])) > 0.5
  for: 10m
  labels:
    severity: warning
  annotations:
    summary: "API p99 latency > 500ms"

- alert: LynxPostgresReplicationLag
  expr: pg_replication_lag_seconds > 30
  for: 5m
  labels:
    severity: warning

- alert: LynxRedisMemoryHigh
  expr: redis_memory_used_bytes / redis_memory_max_bytes > 0.85
  for: 15m
  labels:
    severity: warning
```

### SLO Burn Rate Alerts

Based on [Google SRE Workbook](https://sre.google/workbook/alerting-on-slos/):

```yaml
# 2% error budget consumed in 1h → page
- alert: LynxSLOBurnRateFast
  expr: |
    (1 - (sum(rate(http_requests_total{status!~"5.."}[1h])) / sum(rate(http_requests_total[1h]))) 
    / (1 - 0.999)  # 99.9% availability target
    > 2
  for: 5m
  labels:
    severity: critical

# 10% error budget consumed in 6h → ticket
- alert: LynxSLOBurnRateSlow
  expr: |
    (1 - (sum(rate(http_requests_total{status!~"5.."}[6h])) / sum(rate(http_requests_total[6h]))) 
    / (1 - 0.999)
    > 10
  for: 15m
  labels:
    severity: warning
```

### Alert Routing

```yaml
# alertmanager.yaml
route:
  group_by: ['alertname', 'job']
  group_wait: 30s
  group_interval: 5m
  repeat_interval: 4h
  receiver: 'default'
  routes:
    - match:
        severity: critical
      receiver: 'pagerduty'
      continue: true
    - match:
        severity: warning
      receiver: 'github-issues'
receivers:
  - name: 'pagerduty'
    pagerduty_configs:
      - service_key: '<key>'
        severity: critical
  - name: 'github-issues'
    webhook_configs:
      - url: 'https://github.com/lynx/lynx/alerts'
```

### Runbook Link Requirement

**Every alert MUST have `runbook` label** pointing to `docs/security/RB-XX.md`.
Enforced by CI: `promtool check rules` + custom check for `runbook` label.

## Consequences

### Positive
- **Low noise**: Only actionable alerts page
- **Fast resolution**: Runbook linked, SLO context
- **Auditable**: Alert history in Alertmanager + PagerDuty

### Negative
- **Maintenance**: Rules drift from reality
- **Review**: Quarterly alert audit required

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Alert on every metric threshold | Alert fatigue, no context |
| Only SLO alerts | Misses infrastructure issues (disk, memory) |
| Manual runbook lookup | Slow; link required |

## Related

- ADR 0019: Telemetry (metrics source)
- Security Runbooks (RB-01..RB-10)