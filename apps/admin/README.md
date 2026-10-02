# `apps/admin` — private operations console

**Stack:** Vite · React · TypeScript · Tailwind (admin design tokens, not public tokens).

## Exposure policy — read this first

The admin console is **never internet-exposed**. It is:

- served from a separate hostname on an internal listener,
- reachable only through VPN/zero-trust access,
- authenticated by SSO + MFA (local dev may use basic auth behind an explicit
  `LYNX_ADMIN_ENABLE_BASIC_AUTH=true`, which the production validator rejects),
- excluded from CDN caching, from search indexing (`X-Robots-Tag: noindex, nofollow`),
  and from all public metrics and health checks.

If you cannot reach the admin console from the VPN, that is the intended behaviour.

## Sections

| Section | Contents |
| ------- | -------- |
| Overview | Requests/min, search p50/p95/p99, success rate, index freshness, queue depth |
| Crawler | Fetch rate, bytes, error taxonomy, robots denials, per-host health |
| Queue | Frontier depth by bucket, oldest item age, claim latency, drop reasons |
| Index | Document count, shard state, segment sizes, commit duration, dedup ratio |
| Domains | Crawl status, robots policy, authority, page count, block/unblock |
| Errors | 4xx/5xx histograms, upstream failure classes, top failing paths (paths, not queries) |
| Performance | Latency percentiles per route, indexer throughput, compaction jobs |
| Security | Rate-limit denials, SSRF denials by reason, blocked hosts, audit log |
| Configuration | Effective config (secrets redacted), version, environment, feature flags |

## Rules

- The console renders only aggregated data. It must not be able to display a user's query,
  even for an operator, because the data does not exist to display.
- All mutating actions call `/api/v1/admin/*` and are written to the audit trail
  (`docs/privacy/data-inventory.md`, `admin_audit_log`).
- Destructive actions (index rebuild, purge, domain ban) require a typed confirmation and
  produce an audit entry even on failure.
- Secrets are shown as masked, never returned by the API.