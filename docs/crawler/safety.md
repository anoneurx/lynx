# Crawl safety

The SSRF and resource-exhaustion controls. This is the most security-critical document in
the project.

## Threat model reference

Covers T02 (SSRF to internal services), T12 (decompression bombs), T13 (redirect loops),
T14 (crawler traps), T15 (slowloris), T16 (malicious payloads reaching users), T17 (DNS and
homograph tricks), T18 (robots abuse). Full table:
[threat-model.md §3](../threat-model.md#3-malicious-website-threats-crawl-plane).

## The core rule

> A URL is fetchable only if its **resolved IP address** is publicly routable, at every hop,
> at the moment the socket is opened.

Hostnames are attacker-controlled. Resolved addresses are what the kernel will actually
connect to. Therefore the IP check is authoritative, and hostname-based rules are
defence-in-depth only.

## Validation pipeline

Applied to every URL, before every connection, and again for every redirect hop:

```text
1. scheme      ∈ {http, https}                      else deny(unsupported_scheme)
2. userinfo    absent                               else deny(userinfo_not_allowed)
3. port        ∈ configured allowlist (80, 443)      else deny(port_not_allowed)
4. hostname    non-empty, no NUL, no control chars   else deny(malformed_host)
5. literals    decimal/octal/hex/short forms decoded  else deny(encoded_literal)
6. DNS         resolve A + AAAA                       else deny(dns_failure)
7. addresses   EVERY address passes the deny set     else deny(private_address)
8. pin         the connection uses the validated IP   (no second, unpinned lookup)
9. budget      host not over page/byte budget         else deny(budget_exhausted)
10. politeness  token available, concurrency free     else defer(politeness)
```

Steps 6–8 are what defeat DNS rebinding: we resolve once, check, then *pin* the validated
address for the connection. A second lookup performed by the HTTP client would re-open the
rebinding window.

## Denied address ranges

Policy is **deny-based on every address**, not an allow-list of ranges we thought of.

### IPv4

```text
0.0.0.0/8            this network
10.0.0.0/8           private
100.64.0.0/10        CGNAT
127.0.0.0/8          loopback
169.254.0.0/16       link-local  ← includes 169.254.169.254 metadata
172.16.0.0/12        private
192.0.0.0/24         IETF protocol assignments
192.0.2.0/24         TEST-NET-1
192.88.99.0/24       6to4 relay anycast
192.168.0.0/16       private
198.18.0.0/15        benchmarking
198.51.100.0/24      TEST-NET-2
203.0.113.0/24       TEST-NET-3
224.0.0.0/4          multicast
240.0.0.0/4          reserved
255.255.255.255/32   broadcast
```

### IPv6

```text
::/128              unspecified
::1/128             loopback
::ffff:0:0/96       IPv4-mapped  → unwrap, re-check against the IPv4 list
64:ff9b::/96        NAT64        → unwrap, re-check
2002::/16           6to4         → unwrap embedded IPv4, re-check
fc00::/7            unique local
fe80::/10           link-local
ff00::/8            multicast
::/96               IPv4-compatible (deprecated) → unwrap
```

The unwrap steps matter: `::ffff:127.0.0.1` reaches loopback just as surely as `127.0.0.1`,
and `2002:7f00:0001::` embeds `127.0.0.1` inside a 6to4 prefix. A deny list that misses
these is decoration.

### Hostnames

```text
localhost, *.localhost, *.local, *.internal, *.home.arpa,
metadata.google.internal, metadata.goog, instance-data,
169.254.169.254, fd00:ec2::254 (AWS IMDS), metadata.azure.com
```

**Deny-list only.** A hostname rule that is wrong in the permissive direction does not
matter, because the IP check still refuses the connection. A hostname rule that is wrong in
the restrictive direction costs us coverage — which is why the set is small.

## Redirect handling

Redirects are the classic way to bypass an SSRF filter: the fetch looks safe at hop 0 and
arrives somewhere private at hop 3.

```text
for hop in 0..=max_redirects (5):
    validate(current_url)          ← full pipeline, every hop
    if response is not a redirect: return
    current_url = resolve(base_url, location_header)
    record hop in the chain; detect loops
deny(redirect_budget_exceeded)
```

Additional rules:

- The redirect budget is **5**. Exceeding it is a denial, not a truncation.
- The chain is recorded; a repeat URL in the chain is a loop.
- `Location` is resolved against the current URL per RFC 3986, then re-validated — a
  `Location: //169.254.169.254/` is caught at the next hop exactly like any other.
- Redirects across schemes are allowed only within `{http, https}`; `https → http` is
  permitted (the web is full of it) but the target is re-validated.
- Cookies, `Authorization`, and `Referer` are never carried across a redirect. There is no
  cookie jar, so this is structural.
- The redirect count and final URL are part of the per-URL deadline budget.

## Resource limits

| Limit | Default | Defeats |
| ----- | ------- | ------- |
| `max_response_bytes` | 5 MiB | huge documents, lying `Content-Length` |
| `max_decompression_ratio` | 100:1 | decompression bombs |
| `connect_timeout_ms` | 5 s | unreachable hosts |
| `header_timeout_ms` | 5 s | header-stalling hosts |
| `read_idle_timeout_ms` | 5 s | slowloris mid-body |
| `total_timeout_ms` | 15 s | slow drip within every other limit |
| `max_redirects` | 5 | redirect loops |
| `max_dom_nodes` | 2,000,000 | HTML node explosion |
| `max_dom_depth` | 512 | deep nesting |
| `max_links_per_document` | 50,000 | link farms |
| `max_parse_duration` | 2 s | pathological markup |

Enforcement notes:

- Byte caps are enforced **while streaming**. The body is never fully buffered, and the
  read is aborted the moment the ceiling is crossed — a server that lies about
  `Content-Length` is caught by the streaming counter, not by trusting the header.
- The decompression ratio is computed on the compressed/uncompressed byte counts observed
  during streaming, so a 1 KB body that expands to 1 GB is aborted long before it
  allocates.
- Timeouts are enforced at three levels independently, and the **total** deadline is
  absolute: a slow trickle cannot stay alive by satisfying the idle timeout.
- Limits are configurable but bounded — `crawler.validate()` refuses a maximum above a hard
  ceiling, so a mistake in a config file cannot remove the protection.

## Crawler-trap defence

| Trap | Defence |
| ---- | ------- |
| Infinite URL space (calendar, session ids) | depth cap, per-host page budget, per-host byte budget, parameter-entropy detection |
| `?a=1&b=2` vs `?b=2&a=1` duplicates | query parameters sorted and deduplicated before the dedup key is computed |
| Tracking parameter explosion | a checked-in tracking-parameter list (`utm_*`, `fbclid`, `gclid`, …) is stripped before dedup |
| Link farms | `max_links_per_document`, and outlinks to hosts over the budget are not queued |
| Login/session walls | URLs with credentials in the query are dropped; no cookies are sent, so session walls just yield thin pages |
| Malicious redirects | redirect budget and per-hop validation |
| Zip/quine style infinite bodies | byte and ratio caps |
| Slow responses | idle and total deadlines |
| Very large sitemaps | sitemap size and URL-count caps, sitemap recursion depth cap |

The parameter-normalisation step is subtle and worth stating: without it, a site with `?p=1`
through `?p=999999` and two orderings of a query is 2 million distinct URLs and will
consume the entire budget for one host. Sorting and deduplicating parameters, then
stripping known tracking parameters, collapses that to 999 999 — and the per-host budget
catches the rest. See [packages/common `urlnorm`](../../packages/common/README.md).

## Content handling

What we do with fetched bytes:

- **Never** store raw HTML.
- **Never** render, execute, or sandbox page content.
- Extract text and structured fields, sanitise (strip control characters, zero-width
  characters, bidi overrides), normalise to NFC.
- Results are served as link + title + snippet with
  `rel="noopener noreferrer nofollow"` and `Referrer-Policy: no-referrer`.
- The snippet is HTML-escaped at render time. Snippet content is never injected as markup.

A crawled page cannot cause a request from a user's browser to LYNX, cannot set a cookie
for the LYNX origin, and cannot execute in the LYNX origin. It can only appear as text that
we escape.

## Defence in depth

Three independent layers, because one will eventually have a bug:

| Layer | Control | Failure mode it catches |
| ----- | ------- | ----------------------- |
| Application | `safety.rs` validation pipeline | Logic bugs in our policy |
| Transport | DNS pinning to the validated address | Rebinding between check and connect |
| Network | Kubernetes policy: crawl plane egress limited to 80/443 + DNS, no ingress | A compromised crawler process pivoting internally |

The application layer must exist on its own — the network policy does not protect
local development, CI, or a single-host deployment. The network policy must exist because
the application layer will eventually have a bug.

## Testing

- **Adversarial matrix** (`tests/crawler/safety`): every denied range, every encoding
  (decimal, octal, hex, short, IPv4-mapped IPv6, 6to4, NAT64, mixed case, trailing dot),
  at position 0 and at every redirect position, plus a DNS-rebinding simulation.
- **Fuzzing** (`tests/security` + `fuzz`): the URL parser, the redirect resolver, the
  streaming cap logic. A property test asserts that no input produces a fetch to a denied
  address.
- **Fixture origin server** serves the hostile corpus: lying `Content-Length`, gzip bombs,
  redirect chains to private addresses, infinite redirect loops, slowloris, calendars,
  100 000-link pages, 500 000-node documents.
- All tests are hermetic. CI never contacts the public internet.

## Residual risk

- A DNS or application-layer proxy the crawler cannot see could forward to an internal
  address. We cannot detect this; we minimise exposure by refusing hosts that resolve to
  any non-public address.
- Traps that vary per request evade per-parameter heuristics; per-host budgets contain them
  at the cost of coverage on that host.
- A response that trickles bytes slower than every deadline without crossing them can hold
  a worker for the total deadline. Bounded by `total_timeout_ms` and concurrency limits.
- IPv6 coverage on the public internet is uneven, so a site reachable only over IPv6 may
  be missed. Coverage cost, not a security gap.