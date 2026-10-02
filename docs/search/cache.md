# Response caching

Caching is where a search engine's privacy posture usually leaks, because the cache key
almost always *is* the query. LYNX caches responses but never treats a query as an identifier.

```text
request ──▶ canonicalise plan ──▶ hash ──▶ cache lookup
                                            │ miss
                                            ▼
                                        execute
                                            │
                                            ▼
                                    store (hashed key, bounded value)
```

## What is cached

| Layer | Key | TTL | Contents |
| ----- | --- | --- | --- |
| Response cache (Redis) | `HMAC-SHA256(server_secret, canonical_plan)`, truncated to 128 bits | 300 s, 60 s after an index swap | Rendered result payload |
| Query-plan cache | Same key, plan serialisation | 600 s | Parsed and analysed plan |
| Prefix dictionary | n/a (in-memory per shard) | Rebuilt per generation | Autocomplete dictionary |
| Document cache | `doc_id` | Per generation | Parsed doc used for snippetting |
| Negative cache | Plan hash | 30 s | Empty result sets |

Note the key type for document cache: `doc_id`. Not a URL, not a query. That is the difference
between a cache and a query log.

## What is never cached

| Never | Why |
| ----- | --- |
| The raw query string as a key component | It would make Redis a query log, and a breach would become a data breach rather than a performance incident |
| Session or user identifiers | Nothing is per-user, so nothing keys on a user |
| Personalised anything | No personalisation exists to cache |
| Request headers, cookies, IP | Never part of a key |
| An uncensored rendering of a blocked result | Cached payloads respect the same rules as live ones |

## Key derivation

```text
canonical_plan = serialise(QueryPlan with volatile fields normalised)
                  + index_generation
                  + ranking_config_version
                  + interface_language
                  + result_count

key = HMAC-SHA256(server_cache_secret, canonical_plan)[0..16]
```

Four things make this work:

- **The plan, not the string.** `"rust async"` and `rust async` and `rust  async` are one
  cache entry. A plain SHA of the raw string would create thousands of entries for one query.
- **HMAC, not a bare hash.** An attacker cannot precompute which keys exist, and cannot
  confirm a guess without the secret. Bare hashes of a low-entropy key space are enumerable.
- **Generation in the key.** A new index generation invalidates every entry automatically,
  with no invalidation protocol to get wrong. The 60-second secondary TTL exists only to
  absorb a rapid sequence of swaps.
- **Config version in the key.** A ranking change takes effect immediately.

## Privacy analysis

The cache is not a query log, and it is worth being precise about why:

| Property | Mechanism |
| -------- | --------- |
| No query recoverable from a key | HMAC is one-way; the secret never leaves the process environment |
| No cross-user linkability | Every user of a query produces the same key; the cache stores one payload, not N entries |
| Bounded retention | TTL, with an absolute cap; nothing survives indefinitely |
| Bounded size | Redis `maxmemory` with `allkeys-lru`; eviction is a performance decision, never a correctness one |
| No encryption requirement | Values are public result data; the threat is a key-enumeration attack, which HMAC addresses |
| A dump is not a user history | A Redis dump reveals popular queries by key count, not who searched what. Popularity is visible in every search engine's page counts anyway |

The residual risk is stated rather than dismissed: a Redis dump plus the cache secret would
allow confirming a query guess. The mitigation is that the secret is never stored alongside the
cache, the cache holds only public result data, and TTL bounds the window. If the secret
leaked, the exposure is "which popular searches were cached recently" — not a user history.

## Invalidation

```text
index generation swap  → keys change (generation is in the key) → implicit full invalidation
ranking config change  → keys change (version is in the key)     → implicit full invalidation
secret rotation        → keys change                               → implicit full invalidation
TTL expiry             → bounded staleness, 5 minutes maximum
manual purge           → FLUSHDB, used on a suspected incident
```

Every invalidation event is implicit in the key. There is no tag-based scheme, no
generation registry, and no cache-purge code path — the mechanisms that cause "stale results
that nobody can explain" are absent by construction.

## Stampede protection

```text
1  first request for a key misses
2  it acquires a short lock (Redis SET NX PX 2000)
3  it executes and stores the payload
4  concurrent requests for the same key either wait briefly or compute their own
   (correctness never depends on the lock)
```

The lock is an optimisation, not a correctness mechanism. If it is lost or unavailable, the
consequence is duplicated work, never a wrong or missing result.

## Sizing and protection

```text
maxmemory             2 GiB per node
policy                allkeys-lru
key TTL               300 s
value size cap        64 KiB   (a 200-result payload would be a bug; cap it anyway)
pipeline, no per-key round trips beyond the one MGET
connection pool       bounded; no unbounded fan-out to Redis
```

A value-size cap is the cheap defence against a pathological query producing a megabyte of
payload, and it turns an unbounded response into a bounded one.

## Testing

- **Privacy:** a test asserting a raw query string never appears as a Redis key — the cache
  client is the single choke point, and the test inspects keys written during a scripted
  search.
- **Correctness:** identical plans with different whitespace share one entry; different
  generations do not.
- **Freshness:** after a generation swap, the next request misses and recomputes.
- **Stampede:** 200 concurrent requests for one uncached plan produce one execution and 200
  correct responses.
- **Bounds:** a plan producing an oversized payload is capped, not rejected.
- **Secret rotation:** after rotating the secret, every key changes and nothing is served from
  the old namespace.