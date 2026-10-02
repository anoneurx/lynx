# RB-10 · Disk pressure

**Severity:** SEV2 · **Owner:** operations

Disk exhaustion in a search engine degrades gracefully but eventually stops, and it stops in a
way that looks like index corruption. Handle it before it reaches that point.

## First action

```bash
df -h /var/lib/lynx
lynx-index status --disk-usage | jq '{generation, bytes_by_shard, growth_rate_daily}'
```

## Where the space goes

| Consumer | Growth | Controllable |
| -------- | ------ | ------------ |
| Live Tantivy segments | Corpus growth, ~15–25 %/year | By crawl volume |
| Old index generations | One full copy per generation | Yes — retention count |
| Segment merge temporaries | Up to 2× the shard during a merge | Transient, but a spike |
| Postgres | Tombstones, frontier, link graph, WAL | By policy |
| Journald logs | 7 d retention | Yes — lower the retention |
| Docker layers | Build cache | Yes — prune |

Old generations are the usual answer, because they are the easiest to reclaim and the least
likely to be needed. Keeping two previous generations is the documented policy, and a system
with five is wasting a full corpus copy each.

## Reclaim

In order of preference:

```text
1  delete generations beyond the retention policy     — no user impact
2  prune Docker build cache                          — no user impact
3  lower the journald retention                      — small operational impact
4  VACUUM ANALYZE on Postgres                        — brief lock
5  force a segment merge on the largest shard        — costly, do out of hours
6  reduce the crawl rate                             — reduces index growth
7  delete the oldest documents                        — quality impact, last resort
```

Step 7 is the only one with a user-visible cost, which is why it is last. Reducing the crawl
rate achieves the same goal over a longer horizon without discarding content.

## Merge pressure

A merge needs temporary space. If a merge cannot start for lack of space, the shard stalls and
search results for that shard age, which looks like [RB-01](search-unavailable.md).

```text
ensure free space ≥ 2× the largest shard BEFORE scheduling merges
never merge as an emergency response to low disk — that makes it worse
```

This is why step 5 in the reclaim list is qualified: merging under disk pressure can convert a
SEV2 into an outage.

## Prevent

| Control | Threshold | Action |
| ------- | --------- | ------ |
| Disk usage alert | 75 % | Page; investigate growth |
| Disk usage alert | 85 % | Page; reclaim generations now |
| Growth rate alert | Projected full within 48 h | Reclaim and reduce the crawl rate |
| Merge headroom check | Less than 2× shard free | Block scheduled merges, do not start one |
| Generation retention | 2 previous | Automatic cleanup; alert if exceeded |

Alerting on projected time-to-full rather than only on current usage is what buys lead time. A
75 % threshold on a fast-filling disk can be hours of warning, which is not enough; the growth
rate gives days.

## Verify

```bash
df -h /var/lib/lynx                                    # > 25 % free
lynx-index status --disk-usage | jq '.bytes_by_shard'  # as expected
lynx-index verify --generation <live> --manifest       # still valid
lynx-eval golden --subset integrity                    # results unchanged
```

The last two matter because reclaiming space is the operation most likely to delete something
still in use.

## Escalate

- Cannot reclaim below 85 % → add capacity; this is a capacity-planning failure, not an
  incident, and it needs a [scaling](../../operations/scaling.md) decision
- Documents deleted to reclaim space → SEV2, since index quality is now user-visible
- Recurring pressure → review the crawl volume policy; sustainable ingest may be below what the
  frontier is configured to attempt