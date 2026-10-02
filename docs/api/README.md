# API

The HTTP surface. Seven endpoints, no accounts, no sessions, no cookies.

| Document | Contents |
| -------- | -------- |
| [openapi.yaml](./openapi.yaml) | The machine-readable contract; everything else describes it |
| [search.md](./search.md) | `GET /search`, streaming, operators, pagination |
| [suggestions.md](./suggestions.md) | `GET /suggestions`, typeahead contract |
| [status.md](./status.md) | `GET /status`, freshness, health |
| [explain.md](./explain.md) | `GET /explain`, the ranking breakdown |
| [errors.md](./errors.md) | Error envelope, codes, status mapping |
| [rate-limits.md](./rate-limits.md) | Token buckets, headers, key scopes |
| [versioning.md](./versioning.md) | Stability policy, deprecation, compatibility |

The user-facing summary is [API.md](../../API.md).

## Endpoints

| Method | Path | Auth | Purpose |
| ------ | ---- | ---- | ------- |
| `GET` | `/api/v1/search` | none | Search |
| `GET` | `/api/v1/suggestions` | none | Typeahead |
| `GET` | `/api/v1/status` | none | Index freshness and health |
| `GET` | `/api/v1/documents/{doc_id}` | none | Public metadata for an opaque id |
| `GET` | `/api/v1/explain` | operator | Ranking breakdown |
| `POST` | `/api/v1/keys` | operator | Create a developer key |
| `GET` | `/api/v1/keys` | operator | List keys |
| `DELETE` | `/api/v1/keys/{id}` | operator | Revoke a key |
| `*` | `/api/v1/admin/*` | VPN + SSO | Operational APIs, never internet-exposed |

The first four need no authentication. That is deliberate: a search engine that requires a key
to search is not a search engine, and requiring registration would create a user store, which
would break [../privacy/privacy-model.md](../privacy/privacy-model.md).

## Design principles

1. **No authentication for public search.** No accounts, no cookies, no sessions.
2. **The query never leaves the process.** No endpoint writes one anywhere.
3. **Errors are informative.** Every error says what happened and what to do, in a stable
   machine-readable code.
4. **Partial results are a success.** A degraded shard returns results with a notice, not a
   500.
5. **JSON by default, HTML for browsers.** `Accept` selects the representation; no
   client-side framework needed for first paint.
6. **The API version is in the path**, and `v1` is stable once shipped.
7. **Rate limits are visible.** Every response carries the limit headers, so a client never has
   to guess why it was throttled.

## Example

```bash
curl -sG 'https://lynx.example/api/v1/search' \
  --data-urlencode 'q=rust async runtime' \
  --data 'count=10' \
  --data 'format=json' | jq
```

```json
{
  "query": { "terms": 3, "intent": "informational", "language": "en",
             "corrections": [], "expansions": [] },
  "results": [
    { "rank": 1,
      "doc_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV",
      "url": "https://tokio.rs/",
      "title": "Tokio - An asynchronous runtime for Rust",
      "snippet": "A multi-threaded asynchronous runtime for Rust, with a scheduler, timers, and an I/O driver.",
      "score": 12.4817,
      "signals": { "lexical": 8.42, "freshness": 0.83, "quality": 0.61,
                   "authority": 0.51, "context": 1.0, "spam": 0.0 },
      "cluster_id": null }
  ],
  "meta": { "total": 1841, "returned": 10, "took_ms": 41,
            "index_generation": "gen-2026-02-17T04:00:00Z",
            "ranking_config_version": "v0.1.0",
            "degraded": false, "notice": null }
}
```

The `signals` block is returned by default for the JSON representation. It costs nothing — the
values are already computed during ranking — and it makes the API self-documenting about how it
ranks, which is the same commitment [../ranking/explainability.md](../ranking/explainability.md)
makes for operators.

## Authentication

| Surface | Mechanism |
| ------- | --------- |
| Public endpoints | None |
| `/explain` | Operator token, [../security/key-management.md](../security/key-management.md) |
| `/keys` | Operator token, plus the ability to create keys |
| `/admin/*` | VPN path plus SSO, never exposed to the internet |

API keys exist for rate limiting and attribution of *developer* traffic, not for users. A key is
opaque, scoped, revocable, and holds no personal data:

```json
{ "id": "key_01JX…", "prefix": "lynx_ab12", "scopes": ["search", "suggestions"],
  "rate_limit": { "requests_per_minute": 60 },
  "created_at": "2026-02-17T04:00:00Z", "last_used_at": "2026-02-17T11:22:03Z",
  "revoked": false }
```

The secret is returned exactly once at creation and is never stored in retrievable form — only
a hash, as with any credential. Key usage is aggregated by key, never by query: no per-query log
exists even for keys.

## Streaming

```bash
curl -N 'https://lynx.example/api/v1/search?q=rust&stream=true'
```

Server-sent events, one event per result batch:

```text
event: meta
data: {"index_generation":"gen-2026-02-17T04:00:00Z","candidate_count":1841}

event: results
data: [{"rank":1,"doc_id":"01JX…","title":"Tokio",…}]

event: results
data: [{"rank":4,"doc_id":"01JX…","title":"…",…}]

event: done
data: {"returned":10,"took_ms":41}
```

Streaming is what keeps p50 low even when p95 is not: the first results are in the first
30–40 ms, and the user sees them while slower shards are still being queried. See
[../search/README.md](../search/README.md).

## Content negotiation

| `Accept` | Response |
| -------- | -------- |
| `application/json` | JSON |
| `text/html` | Server-rendered HTML, progressive enhancement |
| `text/event-stream` | SSE, with `stream=true` |
| Anything else | JSON, with a `Warning` header |

No HTML is served to a browser from the JSON representation, and no JSON is embedded in HTML.
The HTML path is rendered from the same data structure, so the two cannot diverge.

## Privacy properties, restated at the API layer

| Property | Mechanism |
| -------- | --------- |
| No query logging | No persistence call exists in any handler |
| No cookies | `Set-Cookie` is never sent |
| No third-party requests | Egress allowlist; CSP on the HTML path |
| No IP retention | Bucketed with a daily-rotating salt for rate limiting only |
| No user agent retention | Mapped to a coarse class at the edge and discarded |
| No sessions | No session cookie, no session store, no session header |
| Reference to a document does not reveal a query | `doc_id` is opaque; it does not encode the query |

The last row is easy to get wrong: if `doc_id` were derived from the query, then a `/documents/`
call would leak the query. It is a ULID, so it leaks only a creation timestamp.

## Related

- [API.md](../../API.md) — the user-facing summary
- [../search/](../search/) — how a query becomes a response
- [../security/rate-limits.md](../security/) — abuse protection
- [openapi.yaml](./openapi.yaml) — the contract that generates the client