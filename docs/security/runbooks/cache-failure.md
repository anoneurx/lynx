# RB-08 · Cache failure

**Severity:** SEV3 · **Owner:** operations

The cache is an optimisation with no correctness role, so this is a latency incident and never an
availability one. The response is to bypass it, not to restore it under pressure.

## First action

```bash
lynx-config set --runtime response_cache.enabled false
```

Bypass. Every cached entry is derived from the index, so the results stay correct. Latency rises
by the cache-hit-rate-adjusted cost, and availability is unaffected.

## Why this is safe

| Property | Consequence of bypass |
| -------- | --------------------- |
| Keys are HMAC of a canonical plan | No stale key format can be served |
| Generation and config version are in the key | No stale-content path exists |
| Every entry is recomputable | Nothing is lost |
| No user data is cached | No privacy consequence |

This holds because the cache design has no invalidation protocol to get wrong: generation
changes invalidate implicitly. If bypass were unsafe, the runbook would say so.

## Diagnosis

```bash
lynx-redis info | jq '{used_memory_human, maxmemory_human, evicted_keys,
                        keyspace_hits, keyspace_misses, connected_clients}'
```

| Finding | Cause | Action |
| ------- | ----- | ------ |
| Connection refused | Redis down or wrong address | Keep bypassed; investigate the service |
| `evicted_keys` climbing fast | `maxmemory` too small | Raise `maxmemory`, or shorten the TTL |
| `maxmemory_human` full, hit rate fine | Fragmentation | `maxmemory-policy` set to `noeviction` |
| `connected_clients` at the limit | Client pool exhausted | Find the leak; do not raise the limit blindly |
| Timeouts, low CPU | Network path | Check the network between the app and Redis |
| Hit rate collapsed | Key format changed | Expected after a plan-shape change; verify the new shape |
| Value errors | Serialisation incompatibility after a deploy | Roll back the deploy |

That last row is the one to check first after a deploy, because a serialisation change is the
most likely cause and the cheapest to confirm.

## Restore

```text
1  fix the root cause
2  verify with lynx-redis ping and a write/read round trip
3  re-enable with a short TTL first (30 s)
4  watch the hit rate for 15 minutes
5  return to the configured TTL once stable
```

Step 3 exists because re-enabling with the full TTL while the cause is unproven reintroduces
whatever was evicting entries, now with a warm keyspace to lose.

## Cache stampede

If the hit rate is fine but Redis is saturated after a cache flush:

```text
1  the stampede lock is a lock with a 2 s TTL
2  concurrent requests for one key either wait briefly or compute their own
3  correctness never depends on the lock, so this is safe under any failure
```

If lock contention is visible, verify the lock is failing *open* (compute your own) rather than
closed (wait or error). A cache stampede costs CPU; a closed lock costs availability.

## Verification

```bash
lynx-redis ping
lynx-query explain --query "cache probe" | jq '.timing_ms.cache'    # expect a hit
lynx-eval golden --subset cache                                     # results unchanged
```

## Escalate

- Redis unavailable and bypass insufficient to meet the latency budget → SEV2, scale the search
  process rather than wait for Redis
- Repeated cache incidents → the cache is more load than it is worth; re-evaluate the design
  with latency numbers, and consider caching at the edge with the same key derivation