# ADR 0008: No Query Logging

**Date:** 2024-02-15
**Status:** Accepted
**Deciders:** Privacy Lead, Legal, Tech Lead
**Tags:** privacy, logging, compliance

## Context

Search queries may contain:
- PII (names, addresses, phone numbers)
- Secrets (API keys, passwords accidentally pasted)
- Medical/legal/financial sensitive terms
- Corporate confidential information

Regulations: GDPR, CCPA, LGPD, HIPAA (if applicable).
User trust: "We don't know what you search."

## Decision

**Never log raw query text.** Log only:
- Query hash (BLAKE3, first 8 bytes) for deduplication
- Latency, result count, cache hit/miss
- Error codes (no query context)

### Implementation

```rust
// Structured logging (JSON)
info!(
    "search_completed",
    query_hash = %blake3_hash(query),  // "a1b2c3d4"
    latency_ms = 42,
    results = 10,
    cache_hit = true,
    // NEVER: query = %query
);
```

### Metrics (Prometheus)

```prometheus
# Labels: NO query text
http_requests_total{endpoint="/v1/search", status="200"} 12345
http_request_duration_seconds_bucket{endpoint="/v1/search", le="0.1"} 10000
```

### Exceptions

- **Debug endpoint** (`/debug/search?q=...`) → only on `localhost`, behind feature flag `debug_endpoints`
- **Audit log** for admin API (user management, not search)

## Consequences

### Positive
- **Zero PII risk** in logs/metrics/traces
- **Simplified compliance** (no query data to delete on request)
- **User trust** → competitive advantage

### Negative
- **Harder debugging** → cannot reproduce exact query from logs
- **Mitigation**: Synthetic query generation for load testing; query hash allows grouping

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Log queries, redact PII | Redaction is imperfect; high false negative risk |
| Encrypt queries in logs | Key management complexity; still accessible to ops |
| Sample 1% of queries | Still logs sensitive data; no compliance benefit |

## Related

- ADR 0019: Telemetry (OpenTelemetry, no query text)
- Security Runbook RB-02: Data Exfiltration