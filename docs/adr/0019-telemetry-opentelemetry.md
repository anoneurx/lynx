# ADR 0019: Telemetry (OpenTelemetry)

**Date:** 2024-07-15
**Status:** Accepted
**Deciders:** SRE, Platform Team
**Tags:** telemetry, observability, opentelemetry

## Context

Observability pillars:
- **Metrics** (Prometheus)
- **Logs** (Loki)
- **Traces** (Jaeger)

Need vendor-neutral, standardized instrumentation.

## Decision

**OpenTelemetry (OTel) SDK + Collector** for all services.

### Metrics (Prometheus)

- **SDK**: `opentelemetry-prometheus` exporter → `/metrics` endpoint
- **Standard metrics** (semantic conventions):
  - `http.server.request.duration` (histogram)
  - `http.server.requests` (counter)
  - `db.query.duration` (histogram)
  - `cache.operation.duration` (histogram)
- **Custom metrics** (prefix `lynx.`):
  - `lynx.search.latency` (histogram, attrs: cache_hit, shard)
  - `lynx.crawl.queue_depth` (gauge)
  - `lynx.indexer.generation` (gauge)
  - `lynx.indexer.build_duration` (histogram)

### Logs (Loki)

- **SDK**: `tracing-opentelemetry` + `tracing-subscriber` (JSON)
- **Structured fields** (OTel semantic conventions):
  ```json
  {
    "timestamp": "2026-10-02T12:34:56.789Z",
    "level": "INFO",
    "service.name": "lynx-api",
    "trace_id": "0123456789abcdef",
    "span_id": "abcdef1234567890",
    "message": "search completed",
    "lynx.query_hash": "a1b2c3d4",
    "http.status_code": 200,
    "duration_ms": 42
  }
  ```
- **No query text, no PII** (ADR 0008)

### Traces (Jaeger)

- **SDK**: `opentelemetry-jaeger` exporter → OTel Collector → Jaeger
- **Sampling**: 10% random + 100% errors (tail-based)
- **Propagators**: W3C `traceparent`, `tracestate`
- **Spans**:
  - `http.server` (API entry)
  - `db.query` (SQLx)
  - `cache.operation` (Redis)
  - `crawler.fetch` (HTTP client)
  - `indexer.build` (batch)

### Collector (Sidecar / DaemonSet)

```yaml
# otel-collector-config.yaml
receivers:
  otlp:
    protocols:
      grpc:
      http:
processors:
  batch:
  memory_limiter:
    limit_mib: 512
exporters:
  prometheus:
    endpoint: "0.0.0.0:8889"
  loki:
    endpoint: "http://loki:3100/loki/api/v1/push"
  jaeger:
    endpoint: "jaeger:14250"
    tls:
      insecure: true
service:
  pipelines:
    metrics:
      receivers: [otlp]
      processors: [memory_limiter, batch]
      exporters: [prometheus]
    logs:
      receivers: [otlp]
      processors: [memory_limiter, batch]
      exporters: [loki]
    traces:
      receivers: [otlp]
      processors: [memory_limiter, batch]
      exporters: [jaeger]
```

### Instrumentation Libraries

| Language | Library |
| -------- | ------- |
| Rust | `opentelemetry`, `opentelemetry-prometheus`, `tracing-opentelemetry` |
| Python (eval) | `opentelemetry-instrument` auto-instrumentation |

## Consequences

### Positive
- **Vendor neutral** → switch backends (Datadog, Honeycomb, Grafana Cloud) without code changes
- **Standard semantics** → dashboards/alerts portable
- **Context propagation** → traces across service boundaries
- **Auto-instrumentation** (Python) → zero-code for eval tooling

### Negative
- **Overhead** (~1-2% CPU, ~5% memory) → acceptable
- **Collector ops** → additional component to manage
- **Cardinality risk** → enforce label allowlist in Collector

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Prometheus client only | No traces, no structured logs |
| Datadog agent | Vendor lock-in, cost |
| Custom metrics endpoint | Non-standard, no ecosystem |

## Related

- ADR 0008: No Query Logging (telemetry privacy)
- ADR 0020: Alerting (PrometheusRule)
- Security Runbook RB-03: Service Degradation