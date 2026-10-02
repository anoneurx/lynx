# ADR 0010: Crawler Architecture

**Date:** 2024-03-15
**Status:** Accepted
**Deciders:** Crawl Team, Platform Team
**Tags:** crawler, architecture, politeness

## Context

Crawl requirements:
- 10M+ pages/day
- Politeness: 1 req/2s per domain (configurable)
- Robots.txt compliance
- Deduplication (content hash)
- Fault tolerance (retries, backoff)
- Untrusted HTML parsing (sandboxed)
- Horizontal scaling

## Decision

**Async tokio-based crawler workers** with:
- Redis Streams for queue (priority, consumer groups)
- Redis sorted sets for politeness tokens
- Postgres for metadata + deduplication
- S3 for raw content storage
- `reqwest` + `rustls` for HTTP/2, TLS
- `tlsh`/`simhash` for near-dup detection

### Worker Loop

```rust
loop {
    // 1. Claim batch (Lua: atomic claim + politeness check)
    let batch = claim_batch(&redis, &pg, 64).await?;

    // 2. Fetch concurrently (semaphore: 64 permits)
    let results = fetch_all(batch).await;

    // 3. Parse, extract, deduplicate
    for page in results {
        let content_hash = blake3(&page.body);
        if pg.upsert_page(&page, content_hash).await?.is_new() {
            s3.put(&content_hash, &page.body).await?;
            pg.enqueue_links(&page.links).await?;
        }
    }
}
```

### Politeness

Per-domain token bucket in Redis sorted set:
- `ZADD politeness:example.com <timestamp> <unique_id>`
- `ZREMRANGEBYSCORE politeness:example.com -inf <now - 2000ms>`
- Allow if `ZCARD < burst` (default burst=1)

### Deduplication

- **Exact**: `content_hash` (BLAKE3) unique constraint in Postgres
- **Near-duplicate**: TLSH/Simhash stored in `page_features` table; indexer filters

## Consequences

### Positive
- **High throughput**: 64 concurrent fetches/worker, 200 workers = 12k parallel
- **Politeness guaranteed**: Centralized in Redis, survives worker restarts
- **Resumable**: Queue persists in Redis Streams; claim timeout → re-queue
- **Observability**: Per-domain metrics, queue depth, error categorization

### Negative
- **Redis dependency** for coordination (mitigated: Redis HA)
- **Complexity**: Lua scripts, async correctness, backpressure

## Scaling

| Metric | Scale Up | Scale Down |
| ------ | -------- | ---------- |
| Queue depth/worker > 5k | +10% workers | -10% workers |
| Politeness wait > 5s | +workers (diff domains) | — |
| CPU > 80% | +workers | -workers |

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Scrapy (Python) | GIL limits concurrency; slower; no Rust integration |
| Custom thread pool | Manual async, harder backpressure |
| Headless browser (Playwright) | Too heavy for 10M pages; use only for JS-heavy subset |

## Related

- ADR 0006: Redis for Coordination
- ADR 0007: Separate Planes
- ADR 0017: Sandboxed HTML Parsing