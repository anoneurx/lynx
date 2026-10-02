# `packages/models` — domain types

Domain types shared by the query plane and the crawl plane, and the public API contract
types.

## Layering

```text
domain models (internal)          api DTOs (public)
─────────────────────────         ───────────────────────
Document, DocumentVersion         SearchResponse, SearchResult,
Domain, Url, Link                 SuggestionResponse, StatusResponse,
CrawlFrontier, CrawlAttempt       ApiKeyCreated, ErrorEnvelope
RobotsPolicy, IndexManifest
```

The two layers are separate on purpose. The public API surface is a deliberate, reviewed
narrowing of internal types, so an internal refactor cannot accidentally leak a new field
onto the internet. `#[serde(deny_unknown_fields)]` on inbound types; explicit allow-list
serialisation on outbound types.

## Rules

1. **No user-data types exist in this crate.** There is no `User`, no `Session`, no
   `SearchHistory`. If a PR adds one, the privacy review gate rejects it.
2. Every type crossing a boundary is versioned with a schema tag
   (`#[serde(tag = "v")]` or a version column) so rolling upgrades are safe.
3. Enums are `#[non_exhaustive]` across process boundaries.
4. Anything returned publicly is reviewed against
   [docs/privacy/data-inventory.md](../../docs/privacy/data-inventory.md).

## Notable types

| Type | Purpose |
| ---- | ------- |
| `UrlId` | Stable identity for a canonical URL |
| `Document` | Current indexed state of a URL |
| `DocumentVersion` | One crawl of one URL; content hash + extracted text |
| `Domain` / `Host` | Registrable domain vs. specific host — the distinction robots and budgets need |
| `CrawlFrontierItem` | Queue entry with priority, budget accounting, and denial state |
| `CrawlAttempt` | One fetch: outcome, timing, byte counts, error class |
| `RobotsPolicy` | Parsed, cached, per-user-agent-group rules |
| `IndexManifest` | Which shards, which tokenizer version, which ranking config produced the current index |
| `ParsedDocument` | Parser → indexer contract |
| `QueryAst` / `QueryPlan` | Query processor contract |

## Why `Domain` and `Host` are separate

Politeness and safety decisions are made per *host* (`docs.example.com`), authority and
budget decisions are made per *registrable domain* (`example.com`), and a parked domain
must not inherit a subdomain's crawl budget. Conflating them is a common crawler bug.
Domain grouping uses a public-suffix list, so `example.co.uk` and `example.com` are
distinct. See [docs/crawler/frontier.md](../../docs/crawler/frontier.md).