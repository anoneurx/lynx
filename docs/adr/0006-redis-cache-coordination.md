# ADR 0006: Redis for Cache & Coordination

**Date:** 2024-01-25
**Status:** Accepted
**Deciders:** Platform Team
**Tags:** redis, cache, coordination

## Context

Need:
- Query result cache (sub-millisecond reads)
- Suggest prefix cache
- Crawler politeness tokens (per-domain rate limiting)
- Index reload pub/sub
- Distributed locks (crawler coordination)
- Session/rate-limit state for API

Options: Redis, Memcached, etcd, Consul, in-process (DashMap).

## Decision

**Redis 7 Cluster** (6 shards, 1 replica each).

## Consequences

### Positive
- **Sub-ms latency** → in-memory, async I/O
- **Rich data structures** (sorted sets for politeness, streams for queue, pub/sub)
- **Cluster mode** → horizontal scale, automatic sharding
- **Lua scripts** → atomic multi-step operations (politeness check + decrement)
- **Persistence** (RDB + AOF) → survive restarts for coordination state

### Negative
- **Memory cost** → all hot data in RAM (mitigated: TTL, LRU eviction)
- **Single-threaded per shard** → CPU bound at ~100k ops/s (mitigated: 6 shards)
- **No strong consistency** → async replication (acceptable for cache/coordination)

## Data Patterns

| Use Case | Structure | Key Pattern | TTL | Eviction |
| -------- | --------- | ----------- | --- | -------- |
| Search results | String (JSON) | `search:{blake3(query+opts)}` | 5 min | LRU |
| Suggestions | String (JSON) | `suggest:{prefix}` | 10 min | LRU |
| Politeness | Sorted Set | `politeness:{domain}` | 2 s sliding | TTL |
| Index reload | Pub/Sub | `INDEX_RELOAD` | — | — |
| Crawl coordination | Stream | `crawl:queue` | 7 days | Maxlen |

## Lua Script: Politeness Token Bucket

```lua
-- KEYS[1] = politeness:{domain}
-- ARGV[1] = now_ms, ARGV[2] = rate_per_sec, ARGV[3] = burst
local last = redis.call('ZRANGE', KEYS[1], 0, 0, 'WITHSCORES')
local now = tonumber(ARGV[1])
local rate = tonumber(ARGV[2])
local burst = tonumber(ARGV[3])

if #last == 0 or (now - last[2]) >= (1000 / rate) then
    redis.call('ZADD', KEYS[1], now, now .. '-' .. math.random())
    redis.call('EXPIRE', KEYS[1], math.ceil(burst / rate) + 1)
    return 1  -- allowed
end
return 0  -- rate limited
```

## Sizing

| Shards | Memory/Shard | Total | Use Case |
| ------ | ------------ | ----- | -------- |
| 6 | 16 GiB | 96 GiB | 10M queries/day, 200 crawlers |

## Alternatives Rejected

| Option | Reason |
| ------ | ------ |
| Memcached | No data structures, no pub/sub, no persistence |
| etcd | Higher latency, lower throughput, for metadata not cache |
| Consul | Same as etcd, heavier |
| In-process | Doesn't scale across pods, no persistence |

## Related

- ADR 0004: Index Reload (pub/sub)
- ADR 0010: Crawler Politeness