# `packages/logging` — structured logging

`tracing` setup, JSON output, and the privacy-preserving log policy.

## Output format

JSON to stdout in every deployed environment. Console format is a developer convenience
only, gated on `LYNX_LOG_FORMAT=pretty` and refused when `LYNX_ENV=production`.

```json
{"timestamp":"2026-02-18T09:14:02Z","level":"INFO","target":"lynx::api",
 "request_id":"01J8Z…","span":{"search_request":"…"},"duration_ms":41,
 "route":"/api/v1/search","status":200,"result_count":10,"cache":"miss"}
```

## The query-redaction rule

The crate exposes exactly one way to include query-derived information:

```rust
log.info(shape_of(&query));   // length bucket, token count, operator classes
```

There is no API for logging the query text, and the query handler never holds a `String`
it could pass to a logger — it parses into an AST immediately and the raw string is
dropped. A reviewer can verify this by reading the handler: there is nothing to log.

Log fields that are forbidden outright:

```text
query, q, search_terms, url params with user content, document ids as log fields,
ip addresses, user-agent strings, raw error strings containing a URL query,
```

Error classes are logged as enum variants (`ErrorClass::UpstreamTimeout`), never as the
underlying message, because upstream messages frequently contain the requested URL.

## Rules

- `request_id` is a UUIDv7 generated per request, propagated in the `X-Request-Id`
  response header so a user can quote it without revealing anything.
- Span fields are declared once at the middleware boundary. Adding a span field inside a
  handler that derives from user input requires a second maintainer.
- Query plans are logged in **shape** by default. The full plan is serialised only when
  `debug.explain_plan = true` **and** the caller is authenticated as an operator.
- Sampling: 100 % of errors and rate-limit denials; 1 % of successful search requests by
  default; per-service configurable.
- Log volume is bounded — the log pipeline drops debug lines in production before they
  ever reach disk.