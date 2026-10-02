# robots.txt handling

LYNX respects `robots.txt` as a policy input. Two clarifications that matter:

1. `robots.txt` is **not** a trust input. It does not buy a host extra budget, and a
   permissive `robots.txt` does not permit crawling something our own safety policy
   refuses. See [ADR-0020](../adr/0020-crawl-budget-policy.md).
2. Compliance is measured and reported. A crawler that claims to respect robots and does
   not is worse than one that says it does not try.

## Fetching

```text
first request to host H:
  GET http://H/robots.txt      (scheme follows the target URL; https preferred if available)
  → parse → cache per (host, user_agent_group) → proceed with the URL batch
```

- `robots.txt` is fetched once per host per crawl session and cached in Postgres so every
  worker sees the same policy.
- The fetch goes through the **same safety gate** as any other request: a `robots.txt`
  redirect to `169.254.169.254` is refused exactly like any other.
- The response is capped (256 KiB). A larger `robots.txt` is truncated and logged; it is
  not an error.
- Content type is not checked strictly — some servers send `text/plain`, some `text/html`.
  We parse either as text.
- The body is parsed with a bounded parser: line cap, path-length cap, no regex on
  user-controlled paths without a size bound.

## Parsing

```text
User-agent: A            # consecutive UAs share the next rule block
User-agent: B
Disallow: /path          # single path prefix
Disallow:                # empty = allow all (must be handled correctly)
Allow: /path             # overrides a longer Disallow
Crawl-delay: 10          # non-standard but widely honoured
Sitemap: https://…       # discovery input, not a rule
```

Rule selection, following the de facto standard (the original 1994 draft plus observed
consensus, as implemented by major crawlers):

1. Group records into blocks by `User-agent`.
2. If any block matches our user-agent **exactly**, use only that block. Otherwise use the
   block matching our user-agent **by case-insensitive longest substring match**. If none
   matches, use `*`.
3. Within the chosen block, longest-match wins; `Allow` wins ties against `Disallow`.
4. An empty `Disallow:` matches nothing (it allows everything) — a classic implementation
   bug that would either block a whole site or, inverted, allow everything.

Path matching uses octet-wise prefix comparison with `*` (any sequence) and `$` (end
anchor) wildcards, bounded to a small pattern length. Patterns are compiled once per rule
and cached; there is no per-request regex compilation.

Because `Allow`/`Disallow` patterns are attacker-controlled, they are a ReDoS surface.
Mitigations: bounded pattern length, a hand-rolled matcher rather than a backtracking
regex engine, and fuzz coverage on the matcher.

## User-agent identity

```text
User-Agent: LynxBot/0.1 (+https://anoneurx.com/lynx/bot)
```

Requirements:

- Identifiable, with a URL that explains what LYNX is and how to reach us.
- A stable group token (`LynxBot`) used for rule matching, separate from the version, so
  version bumps do not change our robots group.
- No impersonation. LYNX never claims to be another crawler.
- If a site blocks `LynxBot`, we honour that. We do not rotate user agents to evade a
  block — that would make the identity claim a lie. A site that objects to crawling has a
  mechanism to object, and it works.
- Contact for crawl complaints is published and monitored.

## Crawl-delay

```text
requested = robots Crawl-delay for our group (or *)
effective  = max(requested, crawler.budget.min_inter_request_ms)
```

We take the **larger** of the site's request and our own floor. `Crawl-delay` is
non-standard; honouring it when present and ignoring it when absent is the pragmatic
consensus, and taking the maximum in both cases is strictly more polite.

On receiving a `429` or `503` with `Retry-After`, we set a host cooldown **regardless** of
`robots`, and exponential backoff applies on top of the crawl-delay.

## Caching

| Rule | Value | Rationale |
| ---- | ----- | --------- |
| TTL | 24 h | standard guidance |
| Refresh on 5xx | no | do not re-fetch a robots file we cannot get |
| Refresh on 404 | no, cache the "allow all" outcome for 24 h | prevents robots-fetch amplification |
| Refresh on 2xx | on TTL expiry | |
| Shared | via Postgres, all workers | consistent policy, fewer fetches |
| Per user-agent group | keyed by `(host, group)` | we only ever use one group, but the key is correct |

Caching the negative case matters: without it, a host returning 404 for `robots.txt`
causes one robots fetch per URL, which is an amplification bug against that host.

## Failure behaviour

| Failure | Interpretation | Action |
| ------- | -------------- | ------ |
| 2xx | authoritative | apply rules |
| 404 / 410 | no restrictions published | allow all (subject to our own policies) |
| 401 / 403 | restricted | treat as **disallow all** |
| 429 | rate limited | honour `Retry-After`, host cooldown, defer the batch |
| 5xx | server problem | retry with backoff; treat as **disallow all** until known |
| timeout / connection error | unreachable | retry with backoff; treat as **disallow all** until known |
| malformed / too large | unreadable | treat as **disallow all**, log for review |
| redirect to safety-denied address | attack or misconfiguration | deny, treat as **disallow all** |

The rule is: **any failure we cannot interpret as an explicit permission is treated as
disallow.** That is deliberately conservative. Assuming "no robots = no restrictions" after
a timeout would mean a transient network problem silently becomes permission.

## Verification

- A test suite runs against the fixture origin server with a corpus covering: multiple
  user-agent blocks, longest-match precedence, `Allow` overriding `Disallow`, empty
  `Disallow`, wildcard and `$` patterns, `Crawl-delay`, oversized files, every failure
  status, and redirect chains.
- The compliance metric `lynx_crawler_robots_compliance_ratio` is defined as
  `fetched_allowed / (fetched_allowed + fetched_disallowed_violation)` and must be `1.0`.
  Any value below `1.0` pages immediately.
- A weekly report in `docs/operations/` records compliance, robots fetch failures, and
  the top hosts by denial rate (a denial-rate outlier often means our parser or our
  identification is wrong, and the honest response is to investigate rather than to work
  around it).