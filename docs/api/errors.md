# Errors

One envelope, every endpoint, machine-readable codes, and a message written for a human.

## Envelope

```json
{
  "error": {
    "code": "RATE_LIMITED",
    "message": "Too many requests. Retry after 42 seconds.",
    "status": 429,
    "retryable": true,
    "retry_after_ms": 42000,
    "detail": { "limit": 60, "window_seconds": 60 }
  }
}
```

| Field | Purpose |
| ----- | ------- |
| `code` | Stable, machine-readable, never localised, never reworded |
| `message` | For a human, and specific enough to act on |
| `status` | HTTP status, repeated for clients that only read the body |
| `retryable` | Whether retrying can help |
| `retry_after_ms` | Present when `retryable` |
| `detail` | Endpoint-specific context; contains no query text |

`code` is the contract. `message` is free to be improved; a client must never parse it.

## Codes

| Code | Status | Retryable | Meaning |
| ---- | ------ | --------- | ------- |
| `INVALID_QUERY` | 400 | No | `q` missing, empty, or unparseable after truncation |
| `QUERY_TOO_LONG` | 400 | No | Truncated to 512 bytes; the search still ran |
| `INVALID_PARAMETER` | 400 | No | A parameter failed validation |
| `INVALID_OPERATOR` | 400 | No | An operator's value is malformed; the token was treated literally |
| `UNAUTHORIZED` | 401 | No | Missing or invalid credential |
| `FORBIDDEN` | 403 | No | Valid credential, insufficient scope |
| `NOT_FOUND` | 404 | No | No such resource; not used for empty result sets |
| `METHOD_NOT_ALLOWED` | 405 | No | Wrong method on a known path |
| `RATE_LIMITED` | 429 | Yes | Token bucket exhausted |
| `QUERY_COMPLEXITY_EXCEEDED` | 400 | No | Too many terms after expansion |
| `INDEX_UNAVAILABLE` | 503 | Yes | No generation can be served |
| `SHARD_TIMEOUT` | 200 | n/a | Partial results; the response carries a notice |
| `CACHE_UNAVAILABLE` | 200 | n/a | Serving uncached; the response is correct |
| `INTERNAL` | 500 | Yes | Unhandled; tracked, never detailed to the client |

Two rows are `200` on purpose. A degraded shard and an unavailable cache are both **successful**
searches that happen to be less fresh or slower, and returning 5xx for them would teach clients
to treat a working search as a failure.

## Status mapping

| Situation | Status |
| --------- | ------ |
| Empty result set | **200** with `results: []` |
| Query truncated | **200** with a notice and `truncated: true` |
| Spelling corrected | **200** with `corrections` in the body |
| Some shards degraded | **200** with `degraded: true` and a notice |
| Cache bypassed | **200**, identical results |
| No matches with a filter | **200** with a notice naming the filter |
| Nothing can be served | **200** with a static page, for browsers; **503** for JSON clients |

The last row is the only genuinely ambiguous case. HTML browsers get a readable page with 200
because a 503 produces the browser's own error page, which is worse. JSON clients get 503
because their error handling is programmatic and the distinction is useful.

## Notices versus errors

```text
error     the request could not be fulfilled as asked
notice    the request was fulfilled, with a caveat worth stating
```

| Situation | Form |
| --------- | ---- |
| Query truncated | notice |
| Correction applied | notice |
| Filter produced no matches | notice |
| Shard degraded | notice |
| Index unavailable (HTML) | notice on the fallback page |
| Bad parameter | error |

Keeping these separate is what lets a client distinguish "search worked, with context" from
"search failed", and it is why a notice never appears in the `error` object.

## HTTP semantics

| Convention | Applied |
| ---------- | ------- |
| `Retry-After` | On every 429 and 503, in seconds |
| `X-Request-Id` | On every response, correlating logs and traces |
| `Cache-Control` | `no-store` on errors, so a transient failure is not cached |
| `Vary` | `Accept, Accept-Language` on all responses |
| Status reuse | 400 for any client input problem, 5xx only for server faults |

The status column is intentionally coarse. A client that branches on 4xx versus 5xx gets the
distinction it needs, and branching on individual statuses is what creates clients that break when
a new code is added.

## Stability

| Aspect | Policy |
| ------ | ------ |
| `code` | Stable. A code's meaning never changes within a major version |
| New codes | May be added in a minor version; a client must treat an unknown code as `retryable: false` and display `message` |
| `message` | May be reworded at any time. Never parse it |
| `detail` keys | May be added. Never required |
| Removal | Only at a major version, with a deprecation period |

Treating an unknown code as non-retryable is the safe default, because retrying an unrecognised
input error is how a client turns a 400 into an outage.

## Privacy in errors

| Rule | Reason |
| ---- | ------ |
| No query text in `message` | An error is often logged or displayed in a context the query did not consent to |
| No query text in `detail` | Same |
| No query text in `X-Request-Id` | It is derived from the request; it must not encode the query |
| No internal stack traces | They contain paths, and occasionally values |
| No hostnames of other systems | An error should not map the internal topology |

Error messages are written to be useful *without* the query, which is a discipline rather than a
convenience. `"Query truncated to 512 bytes at a term boundary"` tells the user and the operator
everything they need.

## Examples

```json
{ "error": { "code": "INVALID_PARAMETER", "message": "count must be between 1 and 100",
             "status": 400, "retryable": false, "detail": { "parameter": "count", "value": 250 } } }
```

```json
{ "error": { "code": "RATE_LIMITED", "message": "Too many requests. Retry after 42 seconds.",
             "status": 429, "retryable": true, "retry_after_ms": 42000,
             "detail": { "limit": 60, "window_seconds": 60 } } }
```

```json
{ "error": { "code": "INDEX_UNAVAILABLE", "message": "No index generation is currently available.",
             "status": 503, "retryable": true, "retry_after_ms": 30000 } }
```

## Related

- [rate-limits.md](./rate-limits.md) — the 429 path in detail
- [status.md](./status.md) — how availability is reported
- [versioning.md](./versioning.md) — the stability policy
- [openapi.yaml](./openapi.yaml) — the codes in the contract