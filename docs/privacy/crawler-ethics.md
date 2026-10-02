# Crawler ethics

The crawler is the part of LYNX that acts on other people's behalf, on other people's
servers. That is a different kind of obligation from serving queries, and it is where a
search engine's ethics are actually decided.

```text
fetch permission is granted in this order:
  1  explicit prohibition     robots.txt Disallow, meta robots noindex, X-Robots-Tag
  2  explicit permission      published sitemap, documented crawler identity
  3  absence of prohibition   crawl, gently
  4  technical refusal        private address space, non-HTTP schemes, hostile ports
```

Precedence is not negotiable: an explicit prohibition at any level beats every signal in the
other direction. A sitemap listing a disallowed URL does not grant permission.

## Compliance

| Mechanism | Implementation |
| -------- | -------------- |
| `robots.txt` | Fetched per host before crawling, honoured per the RFC and the de-facto extensions, cached per the spec's TTL |
| `robots.txt` unavailable | Crawl allowed, at the reduced rate; a 404 is not a prohibition |
| `robots.txt` error (5xx) | Treat as a temporary prohibition for that host — the conservative reading |
| `robots.txt` redirects | Follow up to 5, then treat as unavailable |
| Meta robots `noindex` | Not fetched at all, since the prohibition is known in advance for sitemap URLs |
| `X-Robots-Tag: noindex` | Honoured after fetch; the page is discarded without parsing |
| `Crawl-delay` | Honoured as a floor, and rounded up, never down |
| `Disallow` matching | Longest-match-wins, wildcard and `$` support, byte-range support |
| Crawler identity | A stable, documented, contactable user agent |

The crawler user agent identifies LYNX, links to the project page, and states that LYNX
honours robots.txt. Rotating or disguising it would make LYNX harder to contact and easier to
block, and would be a lie to every server operator.

## Robots.txt is a floor, not a ceiling

Honouring robots.txt is necessary and insufficient. Several cases need more than compliance:

| Case | Additional action |
| ---- | ---------------- |
| Server returns `robots.txt` with a huge file | Cap at 500 KiB per spec guidance; treat as partial |
| `robots.txt` is slow or hangs | 10 s timeout, then treat as a temporary prohibition |
| `robots.txt` changes between crawls | Re-fetch per TTL; a newly added Disallow takes effect immediately |
| A site asks for removal | Honour it permanently; see opt-out below |
| `Crawl-delay` requests a very small value | Round up to the politeness floor, never down |
| A site is rate-limiting or blocking | Back off exponentially and permanently; do not retry aggressively |
| A path pattern suggests non-public content (`/admin`, `/.git`, `/private`) | Never fetch, regardless of robots.txt |

That last row is a policy choice beyond what robots.txt requires, and it is deliberate: a
crawler has no business requesting paths that are named as administrative or private.

## Rate limiting

```text
per-host:      1 concurrent request, default 5 s between requests
per-host-group: same registrable domain shares the bucket   (www and apex are one host)
per-IP-of-target: global cap, to bound fan-out across hosts
per-subnet:    global cap, to bound a misconfigured crawl
concurrency:   512 total across all workers
backoff:       exponential with jitter on 429 or 503, capped at 15 min
```

Sharing the bucket across subdomains of a registrable domain is a deliberate strictness: a
site running a thousand subdomains is one site, and treating them as a thousand hosts would
let LYNX hammer someone who cannot easily see the aggregate rate.

The global caps exist because a crawl frontier can direct every worker at one IP by accident,
and "the per-host limit technically allowed it" is not an acceptable answer to a site
complaining.

## Attribution and feedback

```text
User-Agent: LynxBot/0.1 (+https://lynx.example/bot)
```

The user agent links to a page documenting:

- what LYNX collects and what it does not
- the privacy commitments
- how to opt out (below)
- how to contact a human
- the crawl rate for the host, if identifiable
- how to request deletion of indexed content

A crawler whose operator cannot find out who is crawling their site has no credible privacy
commitment to offer, so the identification must be unambiguous.

## Opt-out

```text
remove.txt in the site root, containing:  User-agent: LynxBot
```

plus, in addition to robots.txt:

- the URL is discovered, validated, and added to a permanent blocklist within one crawl cycle
- blocklist entries are re-verified monthly, so a forgotten opt-out is eventually noticed
- the blocklist is checked at the DNS-resolution stage, so an opted-out host is never contacted
- already-indexed content from that host is removed from the index, not merely deprioritised

Removing content after an opt-out is the part most crawlers get wrong. Leaving indexed content
behind means the opt-out is cosmetic, and the site has no way to verify it worked.

An emailed opt-out request is honoured identically to the file, and acknowledged by a human.

## Legal

| Request | Action |
| ------- | ------ |
| DMCA takedown | Remove the identified content immediately; preserve the notice and the requester; log both |
| Court order | Comply; document the scope; notify affected parties unless prohibited |
| Public-interest request | Comply if lawful and technically possible; document precisely what was disclosed |
| Subpoena for query data | Nothing to disclose; no query data exists |

That last row is worth stating in the public documentation. A site's operator can be confident
that a legal request for "what do people search on your site" produces an answer of nothing,
because there is nothing to hand over. It is one of the few concrete advantages of the privacy
model available to someone other than a user.

## What LYNX does not do to sites

| Not done | Why |
| -------- | --- |
| Rotate user agents or IPs to appear legitimate | Dishonest, and it removes the operator's ability to block |
| Deliberately fetch disallowed URLs "to check" | A prohibition is a prohibition |
| Crawl authenticated areas | No credential handling, ever |
| Follow `javascript:` links or execute scripts | Not a browser; a script runner is a liability |
| Fetch a URL twice to compare content | Deduplication happens after parsing |
| Crawl infinite parameter spaces | URL canonicalisation and depth limits apply |
| Use a site to infer anything about its operator | No profiling of site owners |
| Retry a blocked request repeatedly | Blocking is a decision, not an obstacle |

The pattern across all of these: LYNX does not try to obtain something it was not given.

## Observability and accountability

| Metric | Purpose |
| ------ | ------- |
| Crawl rate per registrable domain, publicly queryable | Any site can see their own rate |
| Blocklist size and growth | Opt-outs being honoured |
| robots.txt failures by host | Operational, and a signal that a site's config is broken |
| 429/503 rate by host | Politeness working, or a host asking for less |
| Blocked requests attempted (should be 0) | A bug indicator: we should never attempt a known-disallowed fetch |

The public crawl-rate endpoint is unusual and deliberate. It makes the crawler's behaviour
auditable by the people it affects, rather than trustworthy by assertion.

## Testing

- Unit tests for robots.txt matching, including wildcard, `$`, byte-range, and longest-match
  cases.
- Precedence tests: sitemap presence does not override `Disallow`; meta robots beats sitemap;
  a retargeted `robots.txt` takes effect on the next cycle.
- Rate-limit tests: subdomain buckets share the parent; the global cap holds under a
  pathological frontier.
- Opt-out test: place `remove.txt` on a test host, verify no contact on the next cycle and
  index removal.
- The blocked-requests-attempted metric is asserted to be zero in the production runbook.