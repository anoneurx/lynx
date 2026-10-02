# RB-02 · Latency spike

**Severity:** SEV2 · **Owner:** operations

## First action

Identify which stage is slow. Do not guess, and do not restart anything yet — a restart
destroys the evidence.

```bash
lynx-query explain --timing --query "warm up the process" | jq '.timing_ms'
```

```json
{ "parse": 1, "cache": 1, "retrieve": 88, "filter": 2, "rank": 11,
  "snippet": 34, "serialise": 3, "total": 140 }
```

## Diagnosis by stage

| Slow stage | Usual cause | Action |
| ---------- | ----------- | ------ |
| `retrieve` | Cold page cache, shard disk saturation, degraded shards being abandoned | §1 |
| `snippet` | Pathological documents, sentence tables paged in from disk | §2 |
| `rank` | Candidate count growth, cluster expansion | §3 |
| `cache` | Redis unreachable or saturated | §4 |
| `parse` | Rare. A regression | Roll back |
| `serialise` | Response size growth, compression CPU | §5 |

### §1 Retrieval

```bash
lynx-status --json | jq '.shards[] | {id, state, p99_retrieve_ms, disk_read_pct}'
```

| Finding | Cause | Action |
| ------- | ----- | ------ |
| All shards slow, `disk_read_pct` > 90 | Index not in page cache | Warm the cache, or add RAM to hold it |
| One shard slow | That shard only | Roll back the generation if it persists |
| `degraded` shards on every query | Deadlines being missed | Investigate the underlying shard; degraded mode adds overhead |
| p99 fine, p50 fine, user reports slow | Not the index | Check edge and network |

### §2 Snippet

```bash
lynx-query explain --query "<query from the report>" | jq '.results[] | .snippet_ms'
```

Oversized documents produce disproportionate snippet cost. The fix is to cap sentence-table
retrieval per result and to keep the total snippet budget enforced.

### §3 Ranking

```text
ranking is bounded: 2000 candidates, 6 signals, cluster suppression
so a ranking slowdown means something is not bounded as designed
```

That is worth taking seriously rather than tuning around: an unbounded cluster expansion or a
cache miss storm in the cluster table indicates a real bug, and the right move is a revert
rather than a raised timeout.

### §4 Cache

| Finding | Cause | Action |
| ------- | ----- | ------ |
| Redis unreachable | Network or process | Bypass the cache ([RB-08](cache-failure.md)); latency rises, availability holds |
| `evicted_keys` climbing | `maxmemory` too small | Raise `maxmemory` |
| Latency high, load low | `maxmemory-policy` thrashing | Switch to `allkeys-lru` if it differs from config |

### §5 Serialisation

Check response size. A change in snippet length or result count shows up here first.

## Mitigation

```text
1  warm the page cache                      reversible, first choice
2  bypass the response cache               reversible, if Redis is the cause
3  lower the candidate budget in config     reversible, small quality cost
4  add replicas for the search process     reversible
5  roll back the last deploy                reversible, if a regression is suspected
6  shed load (rate limit harder)           visible to users, last resort
```

Order matters. Each step is cheaper to reverse than the next, and the first four are invisible
to users.

## Verification

```bash
lynx-bench latency --p50 --p95 --p99 --duration 5m | jq '{p50, p95, p99, budget_ok}'
lynx-status --json | jq '.shards[] | select(.state != "healthy")'   # expect empty
```

## Escalate

- Latency high with no stage attribution → SEV2, engineering investigation
- Cause is index growth rather than a regression → [scaling](../../operations/scaling.md),
  capacity planning
- Recurring at a predictable load → capacity issue, not an incident; schedule it