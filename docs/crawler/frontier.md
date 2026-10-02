# URL frontier

The frontier decides what to crawl next and how much. It is both the crawler's memory and
its throttle: a crawler without frontier discipline is just a load generator pointed at
someone else's server.

## Responsibilities

| Concern | Mechanism |
| ------- | --------- |
| Discovery | Outlinks, sitemaps, RSS/Atom feeds, seed lists, operator submissions |
| Deduplication | Canonical key: scheme + host + normalised path + normalised query |
| Prioritisation | depth, priority hint, staleness, content freshness, host weight |
| Scheduling | Claimable rows by priority, with an age boost so nothing starves |
| Budget input | Per-host page/day and byte/day counters |
| Retry | Classified backoff per error class, with an attempt ceiling |
| Blocking | Per-URL and per-host bans with reason and expiry |
| Budget accounting | Global and per-domain-class budgets, even for URLs we refuse to fetch |

## Data model

```text
crawl_frontier
  url_id         uuid     PK (logical ref, no FK — see docs/database/schema-notes.md)
  state          crawl_state   pending | ready | claimed | done | denied | banned
  priority       real     computed, descending in the claim query
  depth          smallint
  host_id        bigint
  enqueued_at    timestamptz
  not_before     timestamptz   earliest claim time (retry backoff or politeness)
  attempts       smallint
  last_error     error_class
  generation     bigint   index generation this entry targets
  discovery      discovery_source  outlink | sitemap | feed | seed | operator

crawl_budget
  host_id, day          primary key
  pages_fetched, bytes_fetched
  robots_delay_ms, cooldown_until
```

Indexes: `(state, not_before, priority desc) where state in ('pending','ready')`,
`(host_id, state)`, `(url_id)`.

## Deduplication

The dedup key must be *canonical*, or a site can multiply its apparent size for free.

```text
canonical_key(url) = sha256(
    scheme_lower
  + "://" + punycode(host_lower) + strip_trailing_dot
  + (":" + port if port != default_for_scheme)
  + normalise_path(path)                    // collapse //, resolve ./ ../, keep case
  + (if query non-empty) "?" + normalise_query(query)
)
```

`normalise_query` sorts parameter names, deduplicates repeated names with identical values,
drops known tracking parameters (`utm_*`, `fbclid`, `gclid`, `msclkid`, …), drops the
fragment, and drops parameters whose value exceeds a length cap (a common trap vector).

**Without this step** a host with `?p=1..999999` in two parameter orders is 2 million
distinct URLs and will consume an entire host budget on its own. Sorting and deduplicating
is a security-adjacent measure, not a tidiness measure.

Distinct URLs are still stored as distinct rows — dedup is a *queue* property, not a data
loss. Two URLs differing only in a tracking parameter both get crawled and both collapse to
the same canonical URL for indexing.

## Prioritisation

```text
priority = w_depth · (1 / (1 + depth))
         + w_stale · staleness(crawled_at)          // never crawled → 1.0
         + w_hint  · seed_or_operator_hint          // 0 for ordinary outlinks
         + w_fresh · recency_of_promising_signal    // e.g. sitemap lastmod
         + age_boost(now − enqueued_at)             // nothing starves
```

Weights live in `[crawler.priority]` config, not in code. Claiming orders by
`priority desc`; a URL that has waited a long time gains enough age boost to outrank a
newly discovered high-priority URL eventually. Without the age term, a high-volume site
could starve the long tail indefinitely.

## Scheduling and claiming

```sql
-- inside a short transaction
UPDATE crawl_frontier SET state = 'claimed'
WHERE url_id IN (
  SELECT url_id FROM crawl_frontier
  WHERE state = 'ready' AND not_before <= now()
  ORDER BY priority DESC
  FOR UPDATE SKIP LOCKED
  LIMIT $1
)
RETURNING url_id, host_id, depth, priority;
```

`SKIP LOCKED` gives multiple workers contention-free claiming without a broker. The
transaction is deliberately short: it does no I/O beyond the queue itself. Everything slow
(fetch, parse, index) happens outside it.

Claim latency is the metric that decides whether this design still works. If claim latency
or lock contention becomes a bottleneck under measurement, that is the trigger recorded in
[ADR-0008](../adr/0008-frontier-queue.md) to move to a dedicated broker — not a guess made
in advance.

## Politeness interaction

The frontier holds the counters; `services/crawler` enforces them at fetch time.

```text
before claiming a URL for host H:
  robots_delay_ms(H)      → earliest permissible next request
  cooldown_until(H)       → set after repeated failures; overrides everything
  pages_today(H) < max    → else record budget_exhausted and drop
  bytes_today(H) < max    → else record budget_exhausted and drop
  active_for(H) < max_concurrent
```

Budgets are enforced **independently of robots**. A site that returns `Allow: /` gets no
more budget than one that returns nothing, and budget exhaustion drops the URL with a
recorded reason rather than retrying forever. See
[ADR-0020](../adr/0020-crawl-budget-policy.md) and
[politeness.md](./politeness.md).

## Retry and backoff

Classified, because "retry everything 3 times" is how a crawler turns a transient outage
into a self-inflicted denial of service against someone else's host.

| Error class | Retry? | Delay |
| ----------- | ------ | ----- |
| `timeout_connect`, `dns_failure` | yes | exponential from 30 s, ×2, jitter ±25 %, cap 6 h |
| `upstream_5xx` | yes | honours `Retry-After`, else exponential |
| `upstream_429` | yes | honours `Retry-After` **mandatorily**; host cooldown |
| `http_4xx` (not 408/429) | no | permanent; drop |
| `content_too_large`, `unsupported_content_type` | no | permanent; drop |
| `robots_disallow` | no | record `denied` with robots expiry |
| `safety_denied` (SSRF) | no | record; escalate if the host repeats |
| `budget_exhausted` | no | drop for the day |
| `parse_failed` | yes, once | then permanent |
| `index_failed` | yes | retry with backoff (indexing is our bug, not theirs) |

Jitter is mandatory. Without it, every worker retries the same host at the same instant,
which is precisely the behaviour that gets a crawler blocked.

## Bans

| Scope | Trigger | Duration |
| ----- | ------- | -------- |
| URL | permanent 4xx, oversize, unsupported type, safety denial | until the URL changes |
| URL | repeated `parse_failed` | 7 days |
| Host | repeated 5xx/timeouts | escalating: 15 min → 2 h → 24 h → 7 days |
| Host | repeated SSRF denials | immediate escalation; the host is flagged for review |
| Host | explicit abuse report | until reviewed |

Every ban stores a reason and an expiry, and every ban is visible in the admin console.
Bans are a *budget* mechanism as much as a safety one: a broken or hostile host should not
consume a worker's time indefinitely.

## Budget accounting

Budgets are tracked per host per UTC day, with three classes of guard:

1. **Per-host pages/day** — default 5 000.
2. **Per-host bytes/day** — default 5 GiB.
3. **Global daily pages/bytes** — protects the cluster, scaled by deployment size.

Trap detection consumes budget even for refused URLs: a host that generates 100 000
enqueued URLs and we refuse 99 900 still counts the discovery cost, otherwise a trap could
drain the cluster's queue without ever being fetched.

## Operations

| Metric | Meaning |
| ------ | ------- |
| `lynx_frontier_depth{state}` | queue depth by state |
| `lynx_frontier_oldest_age_seconds` | how long the most important waiting item has waited |
| `lynx_frontier_claim_latency_seconds` | time to claim a batch (the ADR-0008 trigger) |
| `lynx_frontier_enqueued_total{source}` | discovery rate by source |
| `lynx_frontier_dropped_total{reason}` | drops by reason, including refusals |
| `lynx_frontier_ban_active{scope}` | active bans by scope |

Alerting: rising `oldest_age_seconds`, `claim_latency` above threshold, drop-reason
distribution changing shape (indicates a new trap pattern), and `budget_exhausted` spikes.