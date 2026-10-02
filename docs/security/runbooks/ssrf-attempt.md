# RB-05 · Suspected SSRF or scanner traffic

**Severity:** SEV1 if any request reached a blocked destination · **Owner:** security

SSRF is one of the few classes where *attempted* means *successful* until proven otherwise, so
the first action is containment rather than diagnosis.

## First action

Confirm whether anything was reached, then block.

```bash
grep -E '"ssrf_blocked"' logs/security.jsonl | jq -r '.url' | sort -u
lynx-audit egress --since 24h | jq '[.[] | select(.allowed == false)]'
```

If `lynx-audit egress` shows any `allowed: true` request to a non-allowlisted destination, treat
it as a successful SSRF and escalate to SEV1 immediately.

## Determine the vector

| Vector | Indicator | Defence to inspect |
| ------ | --------- | ----------------- |
| Crawled redirect | `ssrf_blocked` on a redirect hop | Redirect re-validation, [../ssrf-protection.md](../ssrf-protection.md) step 7 |
| Crawled URL | Blocked at fetch | Steps 1–6 |
| DNS rebinding | Blocked address differs from the resolved one | IP pinning, step 8 |
| AI layer | Any request from the AI process | Astra must have no fetch path at all |
| API proxy | Any request from the API process | The API must have no general egress |
| robots.txt | Blocked while fetching robots | robots.txt is fetched through the same chain |

The redirect vector is the most common, because redirect re-validation is easy to forget when
the initial URL was validated correctly.

## Containment

```bash
# 1. block the source at the edge
lynx-edge block --ip <source> --until 24h

# 2. if the vector is the AI layer, remove the capability
lynx-feature disable astra          # search keeps serving

# 3. if the vector is a compromised crawler host, isolate the plane
lynx-crawler isolate --host <host>
```

Disable the capability rather than debugging it under attack. Astra is optional; the search
plane is the product.

## Verify the defence held

```bash
lynx-audit ssrf --since 24h | jq '
  { attempted: (. | length),
    reached:    ([.[] | select(.connected == true)] | length),
    by_step:    (group_by(.blocked_at) | map({step: .[0].blocked_at, n: length})) }'
```

`reached` must be `0`. If it is not, the control failed and this is a confirmed SSRF: assume
any service reachable from the crawl plane may have been touched.

## Post-incident containment

```text
1  rotate any credential reachable from the crawl plane
   — the crawl plane has no user data, but it has its own credentials
2  review the crawl plane's network routes
3  add a regression fixture with the exact payload
4  add the missing step to the defence chain, or the missing property to the test
5  verify egress allowlist contains no private ranges
```

Rotating credentials even when the destination was unreachable is the conservative call. It is
cheap, and it removes the branch where the reachability assessment was wrong.

## Invariant

```text
no SSRF-blocked request ever reached a socket
```

This invariant already exists and must alert on violation. The purpose of this runbook is to
establish why it fired, not to add monitoring.

## Escalate

- Any request reached a blocked destination → SEV1, security lead, credential rotation
- Source is a crawled page rather than an external attacker → the defence chain has a gap, and
  the content is the vector rather than an attacker
- Cloud metadata reached → SEV1, rotate every cloud credential immediately, and treat the
  network-route check as failed