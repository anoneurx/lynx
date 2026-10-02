# ADR 0012: Sharding Strategy

**Date:** 2024-04-01
**Status:** Accepted (Deferred Implementation)
**Deciders:** Search Team, Architecture Team
**Tags:** sharding, scaling, future

## Context

Tantivy is single-node. At ~1B documents, single index becomes:
- Slow queries (large FST, many segments)
- Long build times (> 24 hrs)
- Large memory footprint per reader
- Single point of failure

Need horizontal sharding strategy before hitting limits.

## Decision

**Hash-based sharding by `url_hash` at application layer.**

### Shard Assignment

```
shard_id = blake3(url) % N
```

- `N` = number of shards (power of 2 for even distribution)
- Deterministic → same URL always maps to same shard
- No coordination needed for routing

### Architecture

```
                    ┌─────────────┐
                    │   Router    │  (in API, stateless)
                    │  shard =    │
                    │ hash(url)%N │
                    └──────┬──────┘
                           │
          ┌────────────────┼────────────────┐
          ▼                ▼                ▼
       ┌──────┐         ┌──────┐         ┌──────┐
       │ API  │         │ API  │         │ API  │  (per-shard reader pools)
       │ S-0  │         │ S-1  │         │ S-N  │
       └──┬───┘         └──┬───┘         └──┬───┘
          │                │                │
          ▼                ▼                ▼
       ┌──────┐         ┌──────┐         ┌──────┐
       │Tantivy│        │Tantivy│        │Tantivy│
       │Idx 0 │         │Idx 1 │         │Idx N │
       └──────┘         └──────┘         └──────┘
```

### Query Execution

1. **Single-shard query** (site:example.com, exact URL): route to one shard
2. **Multi-shard query** (general web search): fan-out to all shards, merge top-K
   - Each shard returns top 100
   - API merges 100×N → top 10 (cheap, N ≤ 64)

### Indexer Per Shard

- N independent indexer leaders (one per shard)
- Each builds its shard's generation
- Publish coordinated via `INDEX_RELOAD` with shard ID

### Rebalancing (Adding Shards)

1. Double N (e.g., 8 → 16)
2. New indexers build new shards (read from old shards' S3 content)
3. Atomic cutover: router switches to new N
4. Old shards deleted after TTL

## Consequences

### Positive
- **Linear scale-out** for index size, query throughput, build time
- **No metadata service** (deterministic routing)
- **Fault isolation** (shard failure = 1/N capacity loss)
- **Independent deploy** per shard

### Negative
- **Cross-shard queries** need merge (latency + complexity)
- **Uneven distribution** if hash skewed (mitigated: BLAKE3 uniform)
- **Schema changes** require all shards rebuild

## When to Implement

| Trigger | Action |
| ------- | ------ |
| Index > 500M docs | Design sharding |
| p99 latency > 500ms | Implement sharding |
| Build time > 24 hrs | Implement sharding |

## Alternatives Rejected

| Strategy | Reason |
| -------- | ------ |
| Range-based (URL prefix) | Skewed (popular domains), hot shards |
| Directory-based (TLD) | Very skewed (.com = 50%+) |
| Quickwit distributed index | Adds separate system; not embedded |

## Related

- ADR 0003: Tantivy
- ADR 0004: Index Generation
- ADR 0011: Indexer Architecture