# `apps/api` — the Search API

**Stack:** Rust · axum · tower · tokio · PostgreSQL · Redis · Tantivy.

This is the only public machine surface of LYNX. It is stateless, horizontally scalable,
and has no outbound internet access at runtime (it reads the index; it does not crawl).

## Route groups

| Prefix | Auth | Purpose |
| ------ | ---- | ------- |
| `/api/v1/search` | none (rate limited) | Web search |
| `/api/v1/suggestions` | none (rate limited) | Query suggestions |
| `/api/v1/status` | none (rate limited) | Index freshness + service health |
| `/api/v1/documents/{id}` | none (rate limited) | Public document metadata (opaque id) |
| `/api/v1/keys/*` | developer key | Self-service API key management |
| `/api/v1/admin/*` | operator/admin + VPN | Operational APIs for the admin console |

Full contract: [API.md](../../API.md) and [docs/api/](../../docs/api/README.md).

## Middleware stack (order matters)

```text
 1. trace_id + request_id propagation      (never contains query text)
 2. security headers                       (CSP, HSTS, X-Content-Type-Options, …)
 3. body size limit + content-type gate
 4. client address extraction              (only if behind a trusted proxy depth > 0)
 5. privacy layer                          (rate-limit bucket, GPC, no-store decisions)
 6. request logging                        (path only; query string redacted)
 7. auth (for /keys and /admin)            (API key hash lookup, or SSO session)
 8. route handler
 9. metrics + tracing spans (shape only)   (no query text as label)
10. response cache lookup (hashed key)     (skip on auth'd, GPC, or no-store routes)
```

## Invariants

- The query string is read **once**, at handler entry, into memory. It is never written to
  a log, a metric label, a trace attribute, an error message, or a database row.
- Every error response is the standard envelope in [docs/api/errors.md](../../docs/api/errors.md),
  carrying a `request_id` that also appears in the response header — enough for a user to
  report a problem without us learning their query.
- The API refuses to start if `privacy.query_logging = true`.
- Admin routes are compiled into the binary but refuse to bind unless
  `admin.require_vpn = true` is explicitly satisfied by the config validator.

## Scaling

- Stateless; scale on CPU and in-flight request count.
- Postgres read replica for anything non-latency-critical; the index reader is a local
  memory-mapped file per replica.
- Response cache is Redis, keyed by `HMAC(cache_salt, normalised_query)` plus an index
  version tag so a new index generation invalidates everything at once.