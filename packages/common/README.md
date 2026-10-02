# `packages/common` — primitives

Shared, dependency-light primitives with no knowledge of LYNX business logic.

## Contents

| Module | Purpose | Notes |
| ------ | ------- | ----- |
| `ids` | UUIDv7 generation, ordering, typed newtypes (`DocumentId`, `UrlId`, `HostId`, `JobId`) | v7 chosen for time-ordered B-tree locality and non-enumerability; see [ADR-0017](../../docs/adr/0017-identifier-strategy.md) |
| `time` | `Instant`-based durations, coarse `UtcNow` provider trait, week/day bucketing | All time-dependent logic takes a clock trait so ranking and freshness are testable and deterministic |
| `errors` | `thiserror` enums per domain, a top-level `ErrorCode`, and the HTTP mapping | Single source of truth for [docs/api/errors.md](../../docs/api/errors.md) |
| `hash` | SipHash/blake3 wrappers, stable content hashing, HMAC helpers | Used for cache keys, dedup signatures, rate-limit buckets |
| `urlnorm` | RFC 3986 normalisation, punycode, default-port removal, query-param sorting, tracking-param stripping, path-segment normalisation | Shared by crawler and API so both agree on what "the same URL" means |
| `text` | Unicode NFC/NFKC, control/zero-width/bidi stripping, width folding, truncation with grapheme safety | All user-facing strings pass through here before logging or rendering |
| `bytesize` | `ByteSize` with parse/format and budget arithmetic | Used by every crawl budget |

## Invariants

- No panics in `Result`-returning paths; the workspace `clippy::panic` lint is set to warn
  and these crates are held to zero warnings.
- All time access goes through `Clock`; `SystemTime::now()` does not appear outside the
  default implementation. This makes freshness and retention logic fully testable.
- `urlnorm` is deterministic and idempotent: `urlnorm(urlnorm(u)) == urlnorm(u)`.
  Property-tested.
- No third-party network access from this crate. Ever.