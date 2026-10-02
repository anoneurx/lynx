# SSRF protection

The crawler's SSRF defences are described in full in [../crawler/safety.md](../crawler/safety.md).
This document covers the **cross-cutting** view: every component that can be induced to make
an outbound request, because SSRF is not only a crawler problem.

## Components that can make requests

| Component | Can it be induced to fetch? | Controls |
| --------- | -------------------------- | -------- |
| `lynx-crawler` | Yes — its purpose | Full defence chain, [../crawler/safety.md](../crawler/safety.md) |
| `lynx-api` | Redirect from a target? No egress at all | Egress allowlist; only internal services reachable |
| `lynx-astra` | **Yes — this is the risk** | See below |
| `lynx-suggestions` | No | Prefix dictionary is in-memory; no fetch path |
| `lynx-snippet` | No | Renders stored text; never fetches |
| Health checks | No | Fixed internal endpoints only |

Two components genuinely can be induced to fetch: the crawler, by design, and the AI layer,
because retrieval is a fetch. The AI layer is the one that is easy to forget.

## The defence chain

Applied in order; any rejection stops the request and the reason is logged.

```text
1  SCHEME       allow http and https only; reject file, gopher, dict, ftp,
                data, javascript, and every other scheme
2  CREDENTIALS  reject any URL with a userinfo component (http://user:pass@host)
3  HOST         reject by name: localhost, *.localhost, *.internal, *.local,
                metadata.google.internal, instance-data, and a maintained denylist
4  RESOLUTION   resolve DNS; reject if ANY returned address fails step 5
5  ADDRESS      reject loopback, private, link-local (incl. 169.254.169.254),
                multicast, broadcast, unspecified, reserved, IPv4-mapped-IPv6,
                NAT64, 6to4, Teredo, documentation ranges
6  PORT         allow 80, 443, 8080, 8443; reject everything else by default
7  REDIRECT     re-run steps 1–6 for every hop; max 5; the validated IP is pinned
8  CONNECTION   bind to the validated address; verify the TLS SNI matches the host
9  BODY         cap 5 MiB, cap decompression ratio at 20:1, cap redirects at 5
```

The three properties that matter most:

**Every address is validated, not just the first.** A host resolving to one public and one
private address fails, because the second is a valid attack and it is trivially forgotten when
only the first is checked.

**The validated IP is pinned for the connection.** Re-resolving between validation and
connect re-opens DNS rebinding: the record that was validated is not the address connected
to. Validation and connection share the address.

**TLS SNI is verified against the hostname** after connecting to the pinned IP, so a
certificate for an internal service cannot be used to satisfy the check.

## Metadata endpoints

The highest-value SSRF target is a cloud metadata service, which hands out credentials.

```text
169.254.169.254    rejected: link-local
fd00:ec2::254     rejected: unique-local
metadata.google.internal   rejected by hostname and by resolved address
100.100.100.200   rejected: Alibaba metadata range, explicitly listed
```

The hostname denylist alone would be insufficient — an attacker could use a public hostname
pointing at the address. The address check is the control; the hostname list exists only to
produce a clearer log message.

Additionally, the crawl plane has **no route to any cloud metadata service** at the network
level. Defence in depth for the highest-severity case: even if every check above failed, the
request has nowhere to go.

## AI-layer SSRF

Astra's retrieval must not become a proxy. Three restrictions, all structural:

| Restriction | Implementation |
| ----------- | -------------- |
| No fetch at query time | Retrieval resolves a `doc_id` from the existing index. No network call in the request path |
| No tool use | The model has no tools, no fetch function, no URL parameter. It emits text only |
| Any future fetch path inherits the full chain | A design rule, so the next person adding a capability cannot skip it |

The first restriction is the real control and it costs something: Astra cannot retrieve
something that is not already indexed, and cannot follow a link. That is acceptable for a
summary feature over the corpus, and it makes the SSRF surface empty rather than filtered.

## DNS handling

| Control | Detail |
| ------- | ------ |
| Resolver | A dedicated resolver with DNSSEC validation where available |
| Response size | Cap 4 KiB; a CNAME chain deeper than 8 is rejected |
| TTL floor | Cached per host for the politeness window, so one crawl cannot amplify DNS queries |
| Validation cache | Validated address cached for the same window, so re-validation is cheap and consistent |
| Rebinding | Impossible by construction: connect uses the validated address |
| Zone transfer | Refused; LYNX is not a secondary for any zone |

## Testing

- Unit tests for every step against a table of addresses: all loopback forms, all private
  forms, link-local, IPv4-mapped IPv6, NAT64, 6to4, Teredo, multicast, and boundary addresses
  such as `0.0.0.0`, `255.255.255.255`, and `169.254.169.255`.
- A mixed-resolution test: a host resolving to one public and one private address must be
  rejected. This is the case most commonly missed.
- Redirect tests: every hop re-validated; a redirect from a public host to `127.0.0.1` and to
  `169.254.169.254` is rejected.
- Scheme tests: `file://`, `gopher://`, `dict://`, `data:`, `javascript:` all rejected before
  any connection attempt.
- Port tests: only the allowlist ports are reachable.
- Credential tests: `http://user:pass@host` rejected.
- Rebinding test: a resolver whose answer changes between validation and connect — the request
  must go to the validated address.
- A network-level test asserting the crawl plane has no route to metadata addresses.

## Related

- [../crawler/safety.md](../crawler/safety.md) — the crawler's full defence chain
- [sandboxing.md](./sandboxing.md) — no network access for the parser at all
- [../threat-model.md](../threat-model.md) — SSRF in the threat register
- [../ai/astra.md](../ai/astra.md) — why the AI layer has no fetch capability