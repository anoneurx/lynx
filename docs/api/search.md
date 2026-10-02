# `GET /api/v1/search`

The endpoint the whole system exists to serve.

## Request

| Parameter | Type | Default | Notes |
| --------- | ---- | ------- | ----- |
| `q` | string | required | The query. Max 512 bytes; truncated at a term boundary with a notice |
| `count` | int | 10 | 1–100 |
| `offset` | int | 0 | Deep pagination is discouraged; see below |
| `format` | enum | `json` | `json` or `html` |
| `stream` | bool | false | SSE when `true` |
| `explain` | bool | false | Include the full signal vector |
| `filters` | string | — | `freshness`, `lang`, `type`, `site`; repeatable |
| `safe` | int | `1` | `1` filters adult content from the index |
| `interface_lang` | string | from `Accept-Language` | UI language, not search language |

Operators go in `q` itself (`site:`, `-`, `intitle:`, `"phrase"`, `after:`,
[../search/query-parsing.md](../search/query-parsing.md)), not in separate parameters. Mixing the
two would produce two grammars with unclear precedence.

## Response

```json
{
  "query": { "terms": 3, "intent": "informational", "language": "en",
             "confidence": 0.96, "corrections": [], "expansions": [],
             "operators_used": [] },
  "results": [
    {
      "rank": 1,
      "doc_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV",
      "url": "https://tokio.rs/",
      "title": "Tokio - An asynchronous runtime for Rust",
      "snippet": "A multi-threaded asynchronous runtime for Rust…",
      "score": 12.4817,
      "published_at": "2025-11-04",
      "content_type": "documentation",
      "signals": { "lexical": 8.42, "freshness": 0.83, "quality": 0.61,
                   "authority": 0.51, "context": 1.0, "spam": 0.0 },
      "cluster_id": null
    }
  ],
  "meta": {
    "total": 1841, "returned": 10, "took_ms": 41,
    "index_generation": "gen-2026-02-17T04:00:00Z",
    "ranking_config_version": "v0.1.0",
    "degraded": false, "notice": null,
    "truncated": false
  }
}
```

## Status codes

| Code | Meaning |
| ---- | ------- |
| 200 | Results, including an empty result set |
| 400 | Missing or unparseable `q` |
| 429 | Rate limited |
| 503 | Index unavailable entirely; the response is still HTML with a notice |

An empty result set is **200 with `results: []`**, not 404. A search that found nothing is a
successful search.

```json
{ "query": { "terms": 1 }, "results": [], "meta": { "total": 0, "notice": null } }
```

When filters caused the emptiness, the notice says so, because the most common complaint about
a search engine is a blank page with no explanation:

```json
{ "meta": { "total": 0,
            "notice": "site:example.com matched 0 documents; the site filter is applied before ranking" } }
```

## Operators

```text
q=rust site:docs.rs intitle:changelog after:2025-01 -github "async runtime"
```

| Operator | Effect |
| -------- | ------ |
| `"…"` | Exact phrase |
| `-term` | Penalty, not exclusion |
| `OR` | Disjunction |
| `site:` | Hard filter |
| `inurl:` | Hard filter |
| `intitle:` | Large weight, not a filter |
| `ext:` / `filetype:` | Hard filter |
| `before:` / `after:` | Hard filter; excludes undated documents |
| `freshness:` | Hard filter |
| `lang:` | Hard filter |
| `+term` | Force-include |

The distinction in rows 3 and 8 is worth restating, because it is the most surprising behaviour
in the API: `-term` penalises rather than excludes, and a date filter excludes undated documents
entirely. Both are documented at
[../search/query-parsing.md](../search/query-parsing.md#filters-vs-scoring), and both produce a
`notice` when they produce an empty result set.

## Corrections

When a correction was applied, the response says so and preserves the original:

```json
{ "query": { "terms": 2,
             "corrections": [ { "from": "pyton", "to": "python", "applied": true } ],
             "expansions": [ { "from": "ml", "to": "machine learning", "weight": 0.4 } ] } }
```

And the HTML representation shows `showing results for python`. The user is told, because silently
changing someone's query is a decision made on their behalf.

## Pagination

`offset` exists and is capped at 1000. Beyond that, results are not served by offset.

```text
why: deep offset requires the ranking to be recomputed and skipped, and skipping on a shifting
     corpus makes page 50 of a result set inconsistent with page 1
what clients should do instead: refine the query, or use filters
```

That is a real limitation and worth stating plainly rather than hiding behind a cursor that
would have the same problem in a different shape.

## Streaming

```bash
curl -N 'https://lynx.example/api/v1/search?q=rust&stream=true'
```

```text
event: meta
data: {"index_generation":"gen-2026-02-17T04:00:00Z","candidate_count":1841}

event: results
data: [{"rank":1,…},{"rank":2,…},{"rank":3,…}]

event: done
data: {"returned":10,"took_ms":41}
```

Results stream as shards resolve. An `event: notice` may arrive at any point carrying a
degraded-mode message, so the client must handle a notice after results have been sent.

## Caching

Response caching is internal and keyed by an HMAC of the canonical plan
([../search/cache.md](../search/cache.md)). Clients see standard headers:

| Header | Meaning |
| ------ | ------- |
| `Cache-Control` | `public, max-age=60` for HTML, `no-store` for `explain` |
| `Age` | Seconds since the internal cache entry was written |
| `ETag` | Hash of `meta.index_generation` + `ranking_config_version` + the plan hash |
| `Vary` | `Accept`, `Accept-Language` |

An `ETag` is stable for a given generation and config version, so a conditional request is cheap
and correct. It does not include the query text, so it leaks nothing.

`no-store` on `/explain` is not a privacy requirement — nothing is retained either way — but
explanations are operator output and should not be cached by intermediaries.

## Rate limits

Headers on every response:

```text
X-RateLimit-Limit: 60
X-RateLimit-Remaining: 57
X-RateLimit-Reset: 42
```

Details in [rate-limits.md](./rate-limits.md). Anonymous clients are limited by bucketed IP;
clients with a key are limited by key.

## Accessibility

The HTML representation is fully usable without JavaScript. With JavaScript, streaming improves
perceived latency but changes nothing about semantics:

- `<ol>` for results, so assistive technology announces "3 of 10"
- Each result is a link with an accessible name from the title, not from the URL
- `aria-busy` on the result list while streaming, cleared on `event: done`
- `aria-live="polite"` on the result count and on notices
- Keyboard access to every control, including operator insertion
- High-contrast mode and a user-selectable font size, both honoured from `prefers-color-scheme`
  and `prefers-contrast`

A search results page that requires a pointer is broken for a meaningful number of people, so
these are requirements rather than enhancements.

## Errors

See [errors.md](./errors.md). The envelope is identical across every endpoint:

```json
{ "error": { "code": "QUERY_TOO_LONG",
             "message": "Query truncated to 512 bytes at a term boundary",
             "detail": { "original_length": 640, "used_length": 508 } } }
```

## Related

- [../search/README.md](../search/README.md) — the request lifecycle
- [../search/query-parsing.md](../search/query-parsing.md) — operator semantics
- [../ranking/explainability.md](../ranking/explainability.md) — the signal vector
- [openapi.yaml](./openapi.yaml) — the generated contract