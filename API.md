# LYNX API

Version: **v1** · Format: JSON · Auth: none for search, Bearer API key for developer
endpoints · Full contract: [docs/api/](./docs/api/README.md) · Spec: `docs/api/openapi.yaml`

## Quick start

```bash
curl -s 'https://lynx.example/api/v1/search?q=rust+search+engine&count=10' | jq
```

```json
{
  "query": "rust search engine",
  "results": [
    {
      "doc_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV",
      "url": "https://example.org/rust-search",
      "display_url": "example.org › rust-search",
      "title": "Building a search engine in Rust",
      "snippet": "…indexing, ranking, and the parts that are harder than they look…",
      "score": 12.4817,
      "signals": {
        "bm25": 9.12,
        "freshness": 0.83,
        "quality": 0.61,
        "authority": 0.44,
        "context": 1.00,
        "spam_penalty": 0.00,
        "duplicate_penalty": 1.00
      },
      "published_at": "2026-01-14T00:00:00Z",
      "crawled_at": "2026-02-11T08:22:00Z",
      "language": "en"
    }
  ],
  "pagination": { "next_cursor": "eyJvIjoxLCJ2IjoxfQ", "has_more": true },
  "took_ms": 38,
  "index": { "generation": "gen-2026-02-17T04:00:00Z", "documents": 4821391 },
  "corrections": [],
  "request_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV"
}
```

## Endpoints

### `GET /api/v1/search`

| Param | Type | Default | Notes |
| ----- | ---- | ------- | ----- |
| `q` | string | required | Query text. Operators supported — see [query-language.md](./docs/search/query-language.md) |
| `count` | int | 10 | 1–50 |
| `cursor` | string | — | Opaque pagination cursor from `next_cursor` |
| `safe` | int | 0 | 0 = all results, 1 = filter explicit-content pages |
| `lang` | string | — | Restrict to a language |
| `freshness` | string | — | `hour` \| `day` \| `week` \| `month` \| `year` |
| `explain` | int | 0 | Include the signal breakdown (always included for operator tokens) |

```bash
curl -sG 'https://lynx.example/api/v1/search' \
  --data-urlencode 'q=site:docs.rs error handling' \
  --data 'count=5' --data 'explain=1'
```

Errors return the standard envelope — see [errors.md](./docs/api/errors.md):

```json
{
  "error": {
    "code": "RATE_LIMITED",
    "message": "Too many requests",
    "request_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV",
    "retry_after_seconds": 42,
    "docs": "https://lynx.example/docs/api#rate-limits"
  }
}
```

### `GET /api/v1/suggestions`

```bash
curl -sG 'https://lynx.example/api/v1/suggestions' --data-urlencode 'q=rust tra' --data 'limit=8'
```

```json
{ "suggestions": ["rust traits", "rust tracing", "rust training"], "took_ms": 3 }
```

Suggestions are derived from the corpus, never from query logs.

### `GET /api/v1/status`

```json
{
  "service": "ok",
  "index": { "generation": "gen-2026-02-17T04:00:00Z", "documents": 4821391, "size_bytes": 1938215732 },
  "crawl": { "frontier_depth": 412883, "oldest_item_age_seconds": 930, "pages_last_hour": 18402 },
  "uptime_seconds": 918233
}
```

### `GET /api/v1/documents/{doc_id}`

Public metadata for an opaque document id — the same fields the SERP shows, plus the full
extracted text. `doc_id` is a UUIDv7 and carries no enumeration meaning.

### `GET /api/v1/explain`

Operator-only. Returns the compiled query plan in shape plus the full signal vector for
each result, so ranking behaviour can be diagnosed without reproducing a query from logs.

## Developer endpoints

Authenticated with `Authorization: Bearer lnx_<32-byte-base32-key>`.

| Method | Path | Purpose |
| ------ | ---- | ------- |
| `POST` | `/api/v1/keys` | Create a key; the secret is returned **once** |
| `GET` | `/api/v1/keys` | List keys (prefix, scopes, created, last used — never the secret) |
| `DELETE` | `/api/v1/keys/{id}` | Revoke immediately |

Keys are stored as `sha256` hashes, are scoped, are rotatable, have a hard monthly quota,
and can be killed individually without affecting other keys. A leaked key costs quota, not
index contents — the public search surface is available to everyone anyway.

## Admin endpoints

`/api/v1/admin/*` requires VPN + SSO. Never internet-exposed. See
[docs/operations/observability.md](./docs/operations/observability.md).

## Rate limits

| Tier | Limit | Window | Keyed by |
| ---- | ----- | ------ | -------- |
| Anonymous | 30 | 1 min | `HMAC(daily_salt, /24 or /48 prefix)` |
| Developer | 300 | 1 min | API key |
| Search endpoint | 600 | 1 min | As above |
| Suggestions | 120 | 1 min | As above |

Response headers:

```text
X-RateLimit-Limit: 30
X-RateLimit-Remaining: 27
X-RateLimit-Reset: 1771412345
Retry-After: 42          # only on 429
```

Buckets rotate daily and expire after 25 hours, so no durable per-user record exists. The
privacy reasoning, and the NAT-sharing cost of this design, are documented in
[PRIVACY.md §4](./PRIVACY.md#4-rate-limiting-without-identity).

## Conventions

- JSON request and response; `Content-Type: application/json`.
- `X-Request-Id` on every response. Quote it in a bug report; it reveals nothing about
  your query.
- Cursors are opaque and stable across index updates; prefer them over page numbers.
- Timestamps are RFC 3339 UTC. Scores are floats with four decimals.
- Breaking changes require a new version prefix. Additive fields may appear without one;
  clients must ignore unknown fields.
- `/healthz`, `/readyz`, and `/metrics` are operational endpoints, not API, and are not
  covered by the version contract.

## Privacy notes for API users

- Queries are not logged. See [PRIVACY.md](./PRIVACY.md).
- `Sec-GPC: 1` disables the response cache for that request.
- The response cache is keyed by an HMAC; a cached response cannot be reversed into a
  query.
- Do not send personal data in queries expecting it to be handled specially. It is not
  stored, not redacted, and not searchable — treat it as ephemeral and do not rely on
  deletion. If you need deletion guarantees, ask before building on top of LYNX.
- Contact [security@anoneurx.com](mailto:security@anoneurx.com) for a DPA before using
  LYNX in a product with your own users.

## Versioning

`/api/v1` is stable once shipped. Deprecations are announced in `CHANGELOG.md` at least
90 days ahead, with a `Deprecation`/`Sunset` header on the affected endpoint. See
[versioning.md](./docs/api/versioning.md).