# `services/frontier` — URL frontier

**Deployment:** v0.1 — worker loop inside the `lynx-crawler` binary. Becomes its own
deployment at Phase 6 when claim contention is measured, not assumed.
Full design: [docs/crawler/frontier.md](../../docs/crawler/frontier.md).

The frontier is the crawler's memory and its throttle. Everything about *what* to crawl
next and *how much* lives here.

## Responsibilities

| Concern | Mechanism |
| ------- | --------- |
| Discovery | Outlinks, sitemaps, RSS/Atom feeds, seed lists, operator-submitted URLs |
| Deduplication | Canonical key = scheme + host + path + normalised query, hashed |
| Prioritisation | `depth`, `priority hint`, `content freshness`, `staleness`, domain weight |
| Scheduling | Claimable rows ordered by priority, with age-based boost so nothing starves |
| Politeness input | Per-host page/day and byte/day budget counters |
| Retry | Classified backoff per failure class, with a hard attempt ceiling |
| Blocking | Per-URL and per-host bans with reasons and expiry |
| Budget accounting | Global, per-domain-class, and per-host crawl budgets |

## State (Postgres in v0.1)

```text
crawl_frontier      url_id PK, state, priority, depth, enqueued_at, not_before,
                    attempts, last_error_class, host_id, generation
crawl_frontier_out  worker claim bookkeeping
crawl_budget        host_id, day, pages_fetched, bytes_fetched, robots_delay_ms
```

Queue implementation is `SELECT … FOR UPDATE SKIP LOCKED` inside a short transaction —
no broker dependency in v0.1. The claim path is a hot spot; the migration trigger to a
dedicated broker is a measured claim-latency threshold, recorded in
[ADR-0008](../../docs/adr/0008-frontier-queue.md).

## Rules

- Frontier state is **never** joined with anything containing user data. It is a crawl
  dataset, not a user dataset.
- A URL that is denied by robots is stored as *denied* with an expiry, not discarded —
  so we do not re-evaluate it on every discovery, and so operators can audit decisions.
- The frontier tracks budget even for URLs it refuses to fetch (trap detection).