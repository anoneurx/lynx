# RB-04 · Crawler stalled

**Severity:** SEV2 if the index is ageing visibly, SEV3 otherwise · **Owner:** crawler operations

## First action

```bash
lynx-crawler status | jq '{workers, frontier_backlog, in_flight, rate_per_second}'
```

A stalled crawler is not immediately user-visible. The relevant question is whether the index
is ageing, so measure the index age before deciding the severity.

## Diagnosis

```text
1  workers alive?              → process problem, §1
2  workers alive, backlog 0?   → the crawler has nothing to do, §2
3  backlog high, rate 0?       → blocked or rate-limited, §3
4  rate > 0, index not growing → writes failing, §4
```

### §1 Workers not alive

| Finding | Cause | Action |
| ------- | ----- | ------ |
| Crash loop | Panic in the fetch path | Read the last trace; a panic in fetch is a bug, restart and investigate |
| OOM kills | Memory limit too low, or a leak | Raise the limit, then investigate; a leak requires a fix, not a bigger limit |
| Seccomp denials | Syscall blocked by the filter | The filter is correct; the code needs review before the filter is relaxed |
| Lock held | Stale lock from a killed process | Clear after confirming no live process holds it |

### §2 Backlog zero

Not an incident by itself. Check why there is nothing to do:

| Finding | Cause | Action |
| ------- | ----- | ------ |
| Frontier genuinely empty, index healthy | Normal early state or exhausted seeds | Add sitemap seeds or raise the frontier budget |
| Frontier empty, index young | Expected during a rebuild | No action |
| Frontier empty, seeds configured | Seed import failed | Re-run the seed import |
| All discovered URLs blocked | A denylist or robots change | Investigate — an accidental global block is a crawl outage |

That last row is the one worth watching: a single over-broad `Disallow` pattern from a
`robots.txt` change can empty the frontier for a host, and it looks identical to exhaustion.

### §3 Blocked or rate-limited

```bash
lynx-crawler metrics | jq '{backoff_hosts, robots_failures, errors_by_code}'
```

| Finding | Cause | Action |
| ------- | ----- | ------ |
| `backoff_hosts` high | Real 429/503 responses | Working as designed; do not disable backoff |
| `errors_by_code.403` high | Blocking | Check the user agent and robots; never rotate identity to evade a block |
| `robots_failures` high | `robots.txt` slow or erroring | Raise the timeout, or accept the conservative treatment |
| `errors_by_code.429` dominant on one host | Politeness too aggressive | Check the configured rate against `Crawl-delay` |
| One host consuming the whole budget | Fan-out | Raise the per-subnet cap only after checking it is not a bug |

The rule under all of these: **do not respond to blocking by evading it.** A block is a
decision, and the response is to back off or to stop.

### §4 Writes failing

| Finding | Cause | Action |
| ------- | ----- | ------ |
| Indexer queue growing | Indexer slower than the crawler | Throttle the crawler, or scale the indexer |
| Disk pressure | Index growth | [RB-10](disk-pressure.md) |
| Postgres errors | Connection limit | Raise `max_connections` or reduce pool size |

## Mitigation

```text
1  restart dead workers                     reversible
2  lower the crawl rate                     reversible, reduces pressure
3  increase indexer capacity                reversible
4  raise per-host politeness limits         NOT reversible in spirit — do not do it as a first step
5  roll back the deploy if a regression is suspected
```

Step 4 is listed to be explicitly rejected: raising politeness limits is how a crawler becomes
a problem for someone else's server, which is the one thing LYNX must never be.

## Verification

```bash
lynx-index status | jq '.documents_crawled_last_hour'   # > 0
lynx-crawler status | jq '.frontier_backlog'            # growing or steady
lynx-status --json | jq .index.median_crawl_age_days    # decreasing
```

## Escalate

- A host complains about crawl volume → the privacy lead, and reduce rather than investigate
- Disk pressure → [RB-10](disk-pressure.md)
- Frontier empty across the whole corpus → the seed set needs work; this is a product issue,
  not an incident