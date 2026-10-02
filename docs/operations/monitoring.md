# Monitoring

## Metrics (Prometheus)

All metrics exposed at `:9090/metrics` (separate port from API).

### Golden signals (per service)

| Service | Latency | Traffic | Errors | Saturation |
| ------- | ------- | ------- | ------ | ---------- |
| `lynx-api` | `http_request_duration_seconds{quantile="0.99"}` | `http_requests_total` | `http_requests_total{status=~"5.."}` | `process_cpu_seconds_total`, `go_goroutines` |
| `lynx-crawler` | `crawler_fetch_duration_seconds` | `crawler_fetches_total` | `crawler_fetches_total{result="error"}` | `crawler_queue_depth`, `crawler_active_workers` |
| `lynx-indexer` | `indexer_build_duration_seconds` | `indexer_documents_indexed_total` | `indexer_build_errors_total` | `indexer_heap_bytes` |
| `lynx-postgres` | `pg_query_duration_seconds` | `pg_xact_commit_total` | `pg_xact_rollback_total` | `pg_database_size_bytes`, `pg_wal_lsn_diff` |

### Key dashboards (Grafana)

| Dashboard | UID | Panels |
| --------- | --- | ------ |
| **LYNX Overview** | `lynx-overview` | QPS, p50/p99 latency, error rate, active crawlers, index generation |
| **API Deep Dive** | `lynx-api` | Latency heatmap, endpoint breakdown, cache hit rate, reader reloads |
| **Crawler** | `lynx-crawler` | Queue depth, fetches/s by status, politeness wait time, DNS/TCP/SSL errors |
| **Indexer** | `lynx-indexer` | Build duration, docs/s, segment count, merge activity, generation lag |
| **Postgres** | `lynx-postgres` | Connections, replication lag, vacuum health, buffer cache hit ratio |
| **Redis** | `lynx-redis` | Memory, evictions, hit rate, pub/sub lag |

Dashboards provisioned via `deploy/grafana/dashboards/`.

## Alerting rules (PrometheusRule)

### Critical (page immediately)

```yaml
- alert: LynxAPIHighErrorRate
  expr: |
    sum(rate(http_requests_total{status=~"5.."}[5m])) by (job)
    /
    sum(rate(http_requests_total[5m])) by (job)
    > 0.05
  for: 2m
  labels:
    severity: critical
  annotations:
    summary: "API 5xx rate > 5%"
    runbook: "https://github.com/lynx/lynx/blob/main/docs/security/RB-01.md"

- alert: LynxCrawlerQueueBacklog
  expr: lynx_crawl_queue_depth > 100000
  for: 10m
  labels:
    severity: critical
  annotations:
    summary: "Crawl queue > 100k URLs for 10 min"
    runbook: "https://github.com/lynx/lynx/blob/main/docs/security/RB-04.md"

- alert: LynxIndexerGenerationStale
  expr: time() - lynx_indexer_last_generation_timestamp > 86400
  for: 5m
  labels:
    severity: critical
  annotations:
    summary: "No new index generation in 24 h"
```

### Warning (ticket, no page)

```yaml
- alert: LynxAPIHighLatency
  expr: histogram_quantile(0.99, rate(http_request_duration_seconds_bucket[5m])) > 0.5
  for: 10m
  labels:
    severity: warning

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

## Logging

Structured JSON to stdout → Loki.

### Standard fields

```json
{
  "timestamp": "2026-10-02T12:34:56.789Z",
  "level": "info",
  "service": "lynx-api",
  "trace_id": "0123456789abcdef",
  "span_id": "abcdef1234567890",
  "msg": "search completed",
  "query_hash": "a1b2c3d4",       // SHA-256(query)[:8], never raw query
  "latency_ms": 42,
  "results": 10,
  "cache_hit": true
}
```

### Log levels

| Level | Usage |
| ----- | ----- |
| `error` | Request failed, action required |
| `warn` | Degraded but functional (fallback used, retry) |
| `info` | Request start/end, config changes, index reload |
| `debug` | Detailed flow (enabled per-pod via `/debug/pprof/`) |

**No query text, IP addresses, or PII in logs.**

## Distributed tracing (Jaeger)

- **Sampling:** 10 % of requests, 100 % of errors.
- **Propagation:** W3C `traceparent` header.
- **Spans:** `http.server` (API), `crawler.fetch`, `indexer.build`, `db.query`, `redis.op`.

## Health endpoints

| Endpoint | Checks | SLA |
| -------- | ------ | --- |
| `GET /healthz` | Process alive | 99.99 % |
| `GET /readyz` | Tantivy reader loaded, Redis reachable, Postgres reachable | 99.9 % |

## Synthetic monitoring

Blackbox prober hits `/search?q=test` every 30 s from 3 regions.
Alert on `probe_success == 0` for 2 min.

## SLOs (quarterly review)

| SLI | Target | Measurement window |
| ----- | ------ | ------------------ |
| Availability (5xx < 0.1 %) | 99.9 % | 28-day rolling |
| Latency p99 < 200 ms | 99.5 % | 28-day rolling |
| Index freshness < 24 h | 99.9 % | 28-day rolling |
| Crawl queue < 10 k | 99.9 % | 28-day rolling |

Error budget burn rate alerts configured per [Google SRE workbook](https://sre.google/workbook/alerting-on-slos/).