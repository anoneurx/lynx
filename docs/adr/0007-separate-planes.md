# ADR 0007: Separate Crawl, Index, Search Planes

**Date:** 2024-02-10
**Status:** Accepted
**Deciders:** Architecture Team
**Tags:** architecture, separation-of-concerns

## Context

Three distinct workloads:
1. **Search** - Read-only, latency-sensitive, stateless
2. **Crawl** - Write-heavy, I/O-bound, untrusted input
3. **Index** - Batch, CPU-intensive, periodic

Running all in one process/deployment causes:
- Crawler OOM → search unavailable
- Indexer CPU spike → search latency spike
- Schema migration → full restart

## Decision

**Three independent services, separate deployments, separate failure domains.**

| Plane | Service | Scaling | Failure Impact |
| ----- | ------- | ------- | -------------- |
| Search | `lynx-api` | HPA (CPU, QPS) | Queries fail (mitigated: multi-AZ) |
| Crawl | `lynx-crawler` | HPA (queue depth) | New pages delayed |
| Index | `lynx-indexer` | Singleton (leader) | Freshness delayed |

### Communication

- **Async only** via Postgres (metadata) + Redis (coordination) + S3 (content)
- **No direct RPC** between planes
- **Search never calls crawl/index** (read-only)

## Consequences

### Positive
- **Isolation**: Crawler memory leak doesn't affect search
- **Independent scaling**: Search scales on QPS, crawl on queue depth
- **Independent deploys**: Indexer rebuild doesn't restart API
- **Security**: Crawler runs with minimal privileges; search read-only
- **Team autonomy**: Search team owns API, crawl team owns crawler

### Negative
- **Operational complexity**: 3+ deployments, 3+ HPA configs
- **Eventual consistency**: Search index lags crawl by minutes
- **Debugging**: Distributed tracing required (Jaeger)

## Data Flow

```
Crawler → Postgres (pages) → S3 (raw HTML)
                ↓
          Indexer (reads PG, S3) → Tantivy segments (EFS)
                ↓
          Redis PUBLISH INDEX_RELOAD
                ↓
          API (reloads reader) → Search results
```

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Monolith | Failure domain too large; scaling granularity poor |
| Search + Index combined | Indexer CPU spikes hurt search latency |
| Crawl + Index combined | Untrusted HTML parsing in indexer process |

## Related

- ADR 0003: Tantivy (search plane)
- ADR 0010: Crawler Architecture
- ADR 0011: Indexer Architecture