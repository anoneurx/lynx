# `tests/crawler` — crawl behaviour tests

All tests run against a **local fixture origin server** that scripts hostile responses. CI
never contacts the public internet.

## The fixture server is a test tool, not a helper

It can be configured to: delay headers, delay body mid-stream, send
`Content-Length: 10` and then 10 GB, send `Content-Encoding: gzip` with a 1 KB payload
expanding to 1 GB, redirect in a loop, redirect to `http://169.254.169.254/`,
`http://127.0.0.1/`, `http://[::1]/`, `file:///etc/passwd`, or a decimal-encoded IP,
return `robots.txt` with traps, serve infinite calendar pages, serve 100 % keyword-stuffed
text, or serve content with 500 000 links.

## Suites

| Suite | Verifies |
| ----- | -------- |
| `safety` | every blocked range is refused, per hop, including after redirect; the IPv4-mapped-IPv6, NAT64, and 6to4 obfuscations are caught |
| `robots` | allow/disallow precedence, wildcards, crawl-delay honoured, `User-agent` grouping, fetch failure semantics, cache TTL |
| `redirects` | budget of 5, loop detection, cross-host redirect re-validation |
| `caps` | byte ceiling, decompression-ratio ceiling, per-phase timeouts, mid-stream abort |
| `traps` | depth cap, per-host page budget, param-space explosion, calendar/link traps |
| `politeness` | per-host token bucket, concurrency cap, `Retry-After` handling, ban escalation |
| `parsing` | malformed HTML, entity expansion, deep nesting, charset fallbacks, huge attribute counts |
| `budget` | byte/page accounting survives restarts and is shared across workers |