# Security Response Headers

The canonical header set applied by every response path, defined once in
[`packages/security/src/headers.rs`](../../packages/security/src/headers.rs) so
the API, admin surface, and any static host cannot drift. The middleware in
`apps/api/src/middleware.rs` applies it centrally — no handler can forget it.

## The set

| Header | Value | Why |
| ------ | ----- | --- |
| `Content-Security-Policy` | `default-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'` | A search API serves documents, not active content. Cascading 'none' is the strongest posture: nothing may load, embed, or submit. |
| `X-Content-Type-Options` | `nosniff` | No client guessing about what a body is. |
| `Referrer-Policy` | `no-referrer` | The search URI is content: a page reached from a result list must not leak it in `Referer`. |
| `X-Frame-Options` | `DENY` | Structural with `frame-ancestors 'none'`; kept for legacy user agents. |
| `Permissions-Policy` | accelerometer/camera/… `=()` | An API requests no device capabilities. |
| `Cross-Origin-Opener-Policy` | `same-origin` | Isolation from any document that somehow opens LYNX. |
| `Cross-Origin-Resource-Policy` | `same-origin` | No other origin may embed LYNX resources. |
| `Cross-Origin-Embedder-Policy` | `require-corp` | Completes the COOP/COEP isolation story. |
| `Cache-Control` | `no-store, no-cache, must-revalidate` | Default stance: responses (and the queries that caused them) persist nowhere. |
| `Pragma` | `no-cache` | HTTP/1.0 back-compat for the same claim. |
| `Strict-Transport-Security` (TLS only) | `max-age=63072000; includeSubDomains` | Emitted **only** when the connection is TLS-terminated with HTTPS-only serving confirmed — HSTS over plain HTTP is a lockout risk at best. |

## The cacheable exceptions

Two surfaces are documented as cacheable and set their own `Cache-Control`,
which the middleware preserves while still applying every other header:

| Surface | Value | Why caching is safe |
| ------- | ----- | ------------------- |
| `/api/v1/suggestions` | `public, max-age=300` | Completions move with the index, not with users; every client's response is identical. `Vary: Accept-Language` is set because the interface language parameter affects the response. |
| `/api/v1/status` | `public, max-age=60` | Aggregate health, identical for everyone. |

Everything else in the system is `no-store`, including every error response —
`429` and `5xx` bodies are not held by shared caches either.

## JSON is the only response shape

The API serves `application/json`; there is no HTML surface on the API plane, and
no useragent-driven content negotiation, so the CSP above is deliberately absolute
rather than tuned-with-allowlists. If a static site later shares the plane, it
gets its own policy document — nothing here mutates to accommodate
script-containing HTML.
