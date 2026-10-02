# `services/` — backend workloads

Each subdirectory is a workspace crate. Three deployment shapes exist, and the split is
deliberate: **no directory here is deployed as its own service in v0.1** unless it says so.
The point of the boundary is that moving a component from "library in one binary" to
"its own deployment" must be a configuration change, not a rewrite.

| Path | Crate | Deployment in v0.1 | Responsibility |
| ---- | ----- | ------------------- | -------------- |
| [`query/`](./query/) | `lynx-query` | library in `apps/api` | Lex, parse, validate, normalise, spell-correct, expand → `QueryPlan` |
| [`ranker/`](./ranker/) | `lynx-ranker` | library in `apps/api` | Feature extraction, weighted scoring, dedup suppression, explain output |
| [`suggestions/`](./suggestions/) | `lynx-suggestions` | library in `apps/api` | Prefix completion from a read-only term/docusFreq replica |
| [`frontier/`](./frontier/) | `lynx-frontier` | library + worker loop in `services/crawler` binary | URL discovery, priority, dedup, scheduling, budget |
| [`parser/`](./parser/) | `lynx-parser` | library in `services/crawler` binary | HTML → text, links, canonical, metadata, language, dates |
| [`indexer/`](./indexer/) | `lynx-indexer` | binary (`lynx-indexer`) | Normalise → tokenise → dedup → write Tantivy docs → commit |
| [`crawler/`](./crawler/) | `lynx-crawler` | binary (`lynx-crawler`), outbound-only network | Fetch execution, SSRF policy, politeness, crawl budgets |
| [`ai/`](./ai/) | `lynx-ai` | binary (`lynx-ai`), optional, egress-restricted | Retrieval-grounded answers with citations (Astra) |

## Two planes

```text
QUERY PLANE (apps/api)          CRAWL PLANE (crawler, indexer, frontier)
─────────────────────────────────  ─────────────────────────────────────────────
internet-facing                   no inbound internet access
reads the index                   outbound only, port 80/443 + DNS
never touches untrusted bytes     the only code that handles hostile input
never sees crawl state            never sees a user query
```

This split is the single most important structural decision in the codebase. A compromise
of the crawl plane does not expose query data, because the crawl plane has no access to
it. See [docs/architecture.md §1](../../docs/architecture.md#1-system-context).

## Interface rule

Crate-to-crate calls go through explicit traits defined in the **consumer**, not the
provider. Concretely: `apps/api` defines the `SearchIndexReader` trait it needs;
`services/indexer` implements it. Nothing in `services/` depends on `apps/`.

```text
apps/api  ──defines trait SearchIndexReader──▶  implemented by services/indexer
apps/api  ──defines trait DocumentSource──────▶  implemented by services/frontier
```

This is what makes the index replaceable (Tantivy → Lucene/OpenSearch → custom) without
touching the ranker or the API. See [ADR-0002](../../docs/adr/0002-search-index.md).