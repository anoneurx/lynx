# Politeness

Being a good citizen of someone else's infrastructure is a hard requirement, not a
courtesy. A crawler that abuses origins is a security and legal problem, and the
engineering consequence is that politeness has to be an enforced budget rather than an
intention.

## Principles

1. **Identify yourself honestly.** Real user agent, a URL explaining what LYNX is, and a
   monitored contact address. No user-agent rotation to evade blocks.
2. **Slow down on request.** A per-host floor between requests, regardless of our capacity.
3. **Respect robots.txt**, including crawl-delay. See [robots.md](./robots.md).
4. **Back off when asked.** `Retry-After` on 429/503 is mandatory and sets a host cooldown
   that overrides everything else.
5. **Have a budget.** Per-host daily pages and bytes, independent of robots.
6. **Stop when a site tells us to.** An explicit block or an abuse report produces a ban
   that a human has to lift.
7. **Distribute load.** Crawls come from a documented, reasonably small set of addresses.
8. **Make it easy to complain** and complain about ourselves when a complaint arrives.

## Per-host controls

```text
max_concurrent_per_host     2
min_inter_request_ms        1000        floor between requests to one host
effective_delay             max(robots_crawl_delay, min_inter_request_ms)
max_pages_per_host_day      5000
max_bytes_per_host_day      5 GiB
cooldown_after_429          Retry-After, or 60 s if absent
cooldown_escalation         15 min → 2 h → 24 h → 7 days on repeated failure
```

Each is a token-bucket or counter held in Postgres (`crawl_budget`) so it is shared across
all crawler workers. An in-memory limiter would reset on every deploy and every restart,
which would quietly erase our politeness history.

## Global controls

| Control | Purpose |
| ------- | ------- |
| Global daily page budget | Bounds cluster cost and total load on the web |
| Global concurrency | Bounds aggregate outbound load |
| Global bytes/day | Bounds bandwidth |
| Frontier depth backpressure | If the queue grows faster than it drains, we crawl *less*, not more aggressively |

Backpressure matters: the instinct when a queue is deep is to add workers. That is correct
for throughput and wrong for politeness. LYNX adds workers up to the global budget and then
lets the queue grow; it never exceeds a host's politeness limits to catch up.

## Adaptive behaviour

LYNX slows down for a host that looks unhappy and speeds up only when everything is healthy:

| Signal | Response |
| ------ | -------- |
| 429 | honour `Retry-After`, set cooldown, exponential backoff on the whole host |
| 503 with `Retry-After` | same |
| Rising 5xx rate | exponential backoff, extend cooldown |
| Rising latency | extend inter-request delay for that host |
| Repeated 4xx | reduce rate (a 404 storm means we are wasting their capacity) |
| Healthy sustained responses | drift the delay back down to the floor, slowly |

The downward drift is deliberately slow. Recovery should be unearned, not automatic.

## Site-specific handling

| Situation | Handling |
| --------- | -------- |
| `noindex` in meta robots | crawl but never index; drop from suggestions |
| `noindex` in `X-Robots-Tag` | same |
| `nofollow` in meta robots | do not queue outlinks, but index the page |
| `Disallow: /` | do not crawl; record denied with robots expiry |
| Site blocks LYNX | honour it; do not rotate identity; do not retry |
| Site asks for a crawl delay increase | increase it |
| Site asks us to leave (abuse report) | ban immediately, human review, documented |
| Login wall | we send no cookies; the page is thin; we index what is visible |
| Rate-limit page returned with 200 | detect known patterns and treat as a rate-limit event |

The last row deserves a note: some sites return a "please slow down" page with a 200
status. Detecting it heuristically (known phrases, tiny content, request-rate challenge
wording) is worth doing, because ignoring it would mean we keep hammering a site that has
explicitly asked us to stop.

## Abuse reports and takedowns

```text
inbound report  →  immediate host/URL ban
                →  audit-log entry
                →  human triage within 24 h (48 h for non-abuse reports)
                →  resolution: uphold → purge + ban; reject → document why
```

We ask for the specific URL when the complaint is about one page, and honour a whole-domain
request without requiring an explanation. Being annoying to someone who wants their content
removed is not a defensible position for a project whose entire claim is that it is
different from the incumbents.

The full policy, including copyright and personal-data handling, is in
[privacy/legal.md](../privacy/legal.md).

## Compliance measurement

| Metric | Target |
| ------ | ------ |
| `lynx_crawler_robots_compliance_ratio` | 1.0 — any violation pages immediately |
| `lynx_crawler_429_rate` | < 0.1 % of requests |
| `lynx_crawler_5xx_rate` | < 1 % |
| `lynx_crawler_avg_inter_request_ms` per host | ≥ the configured floor |
| `lynx_crawler_abuse_reports_open` | 0 beyond the response SLA |

A compliance ratio below 1.0 is treated as an incident, not a metric: it means either our
parser is wrong, our identity is being misread, or there is a code path that bypasses the
policy. All three need a fix, and [docs/security/incident-response.md](../security/incident-response.md)
has the runbook.

## What we do not do

- Rotate user agents, source addresses, or headers to evade a block.
- Crawl behind a proxy list or a residential proxy network.
- Crawl sites that disallow it in order to "improve coverage".
- Bypass authentication, paywalls, or technical access controls.
- Crawl at a rate chosen to finish a benchmark before a deadline.
- Ignore `Retry-After`.

Each of these would improve our metrics and damage the argument that LYNX is a search engine
the web can tolerate.