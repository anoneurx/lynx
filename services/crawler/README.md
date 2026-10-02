# `services/crawler` — fetch execution

**Binary:** `lynx-crawler`. **Deployment:** outbound-only network namespace, no inbound.

The crawler is the only LYNX component that accepts hostile input from the internet. Every
design decision here is a security decision. Full design: [docs/crawler/](../../docs/crawler/README.md).

## Responsibilities

1. Claim URLs from the frontier with bounded concurrency.
2. Enforce per-host politeness (robots, crawl-delay, concurrency, byte budget).
3. Enforce the **fetch safety gate** — the SSRF control — before every socket is opened
   and again at every redirect hop.
4. Fetch over HTTP/1.1 and HTTP/2 using rustls. No cookies, no auth, no JavaScript.
5. Enforce response caps: status, content-type, total bytes, decompression ratio, deadline.
6. Hand the body to `services/parser`, then the extracted document to `services/indexer`
   (or directly into the parse→index pipeline in the same process in v0.1).
7. Feed discovered outlinks back to `services/frontier`.
8. Record every attempt, outcome, and denial reason for operations.

## Non-negotiables

| Rule | Enforcement |
| ---- | ----------- |
| Never open a socket to a private, loopback, link-local, or metadata address | IP-level deny set is authoritative, applied per hop |
| Never follow a redirect without re-running the full validation | Hand-rolled redirect loop, not the client's automatic one |
| Never buffer an unbounded response | Streaming with a hard byte ceiling and mid-stream abort |
| Never send cookies or credentials | No cookie jar exists in this process |
| Never execute or render page content | Text extraction only; no browser engine |
| Never exceed a host's stated budget | Token bucket per host, persistent across workers |

## Files we expect to own

```text
services/crawler/
├── src/
│   ├── main.rs           worker loop, graceful shutdown
│   ├── scheduler.rs      frontier claim loop, concurrency management
│   ├── fetch.rs          HTTP client wrapper, streaming body with caps
│   ├── safety.rs         URL + IP policy, redirect validation      ← security critical
│   ├── net.rs            blocked CIDR sets, DNS resolution + pinning
│   ├── politeness.rs     robots cache, crawl-delay, per-host tokens
│   ├── budget.rs         per-host page/byte budgets, trap detection
│   ├── error.rs          error taxonomy and retry classification
│   └── metrics.rs        counters/histograms (no URL or query labels)
└── benches/              fetch safety and classification benchmarks
```

`safety.rs` and `net.rs` are the two files in LYNX with the strictest review requirements
(two maintainers, explicit security rationale in the PR body, fuzz coverage). See
[docs/crawler/safety.md](../../docs/crawler/safety.md) and
[docs/security/ssrf-defense.md](../../docs/security/ssrf-defense.md).