# ADR 0001: Rust as Primary Implementation Language

**Date:** 2024-01-15
**Status:** Accepted
**Deciders:** Tech Lead, Platform Team
**Tags:** language, architecture

## Context

LYNX requires high throughput, low latency, memory safety, and concurrency for:
- Search API (100k+ QPS, p99 < 200ms)
- Web crawler (millions of concurrent connections)
- Index builder (CPU-intensive, multi-threaded)
- Long-running services (months without restart)

Languages considered: Rust, Go, Java, C++, Python.

## Decision

**Use Rust** for all core services (api, crawler, indexer, ranking).
Python only for evaluation tooling and research scripts.

## Consequences

### Positive
- **Memory safety** without GC pauses → predictable tail latency
- **Zero-cost abstractions** → C++ performance with higher-level ergonomics
- **Fearless concurrency** → data-race-free crawler/indexer at scale
- **Single binary deployment** → distroless containers, fast startup
- **Rich ecosystem** (tokio, tantivy, sqlx, axum, tower)

### Negative
- **Steeper learning curve** → hiring/training investment
- **Longer compile times** → mitigated by sccache, cargo-nextest
- **Fewer libraries** for niche domains → sometimes write custom crates

### Neutral
- Async runtime choice: **tokio** (mature, performant, ecosystem standard)
- No GC means manual memory management for hot paths (e.g., `Arc<Segment>`)

## Alternatives Rejected

| Language | Reason |
| -------- | ------ |
| Go | GC pauses hurt p99; no generics (at decision time); weaker SIMD |
| Java | JVM warmup, memory overhead, GC tuning complexity |
| C++ | Manual memory safety burden; slower iteration |
| Python | Too slow for hot paths; GIL limits concurrency |

## Related

- ADR 0002: Async Runtime (tokio)
- ADR 0003: Tantivy for Search Index