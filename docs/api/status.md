# `GET /api/v1/status`

Index freshness, corpus size, and health. Public, because a search engine's coverage is not a
secret and a status endpoint makes the freshness limits legible.

## Request

No parameters. `?verbose=true` adds per-shard detail for operators.

## Response

```json
{
  "status": "healthy",
  "index": {
    "generation": "gen-2026-02-17T04:00:00Z",
    "generation_age_hours": 4.2,
    "documents": 12847391,
    "bytes": 34359738368,
    "median_crawl_age_days": 3.2,
    "p90_crawl_age_days": 41.0,
    "documents_crawled_last_24h": 184203,
    "shards": 6,
    "last_build_ms": 41231
  },
  "quality": {
    "ranking_config_version": "v0.1.0",
    "avg_quality_score": 0.58,
    "spam_flagged_rate": 0.031,
    "duplicate_rate_top10": 0.02
  },
  "service": {
    "version": "0.1.0",
    "uptime_seconds": 864192,
    "latency_p50_ms": 41,
    "latency_p95_ms": 157,
    "degraded": false
  }
}
```

## Status values

| `status` | Meaning | Client behaviour |
| -------- | ------- | ---------------- |
| `healthy` | Normal | Serve |
| `degraded` | Some shards unavailable; results are partial | Serve, display the notice |
| `maintenance` | Index rebuilding; the previous generation is served | Serve normally |
| `unavailable` | No generation can be served | Serve the static fallback page |

`unavailable` returns HTTP 200 with `status: "unavailable"` rather than a 503. The reason is
that the useful response to "the search engine is down" is a readable page saying so, and a 503
invites a client to retry or to display its own error.

## What is public and what is not

| Public | Operator-only (`verbose=true` with a token) |
| ------ | ------------------------------------------- |
| Document count, index size, generation | Per-shard reader state, disk usage |
| Median and p90 crawl age | Frontier backlog, crawl rate per host |
| Latency percentiles | Worker health, queue depths |
| Quality aggregate scores | Individual spam decisions |
| Version | Anything per-document |

The split is deliberate: corpus-level statistics describe the web, while per-document spam and
cluster decisions are the part of the ranking system that would be worth gaming.

## Why publish this at all

| Reason | Detail |
| ------ | ------ |
| Honest coverage | A user can see that LYNX indexes 12 M documents, and compare |
| Honest freshness | Median age is the single best indicator of whether results are current |
| Debuggability | "The index says p90 age is 41 days" explains a stale-result complaint |
| Verifiable claims | The privacy commitments are checkable with `/status` plus the meta fields |

The freshness numbers are the valuable part. A search engine's quality is bounded by how current
its index is, and publishing the number is more useful than pretending the problem does not exist.

## Caching

```text
Cache-Control: public, max-age=60
```

Short, because a stale status response is worse than a slow one.

## Metrics behind it

| Metric | Source |
| ------ | ------ |
| `lynx_index_documents` | The index manifest |
| `lynx_crawl_age_days{quantile}` | Computed hourly from `document.last_changed_at` |
| `lynx_index_build_duration_seconds` | Indexer |
| `lynx_search_latency_seconds{quantile}` | Request path |
| `lynx_spam_flagged_rate` | Ranking |
| `lynx_dedup_rate_top10` | Ranking |
| `lynx_crawler_documents_total` | Crawler counter |

These are the same values as the Prometheus metrics, rendered for a human. The endpoint exists so
that verifying a claim does not require a Prometheus installation.

## Privacy

```text
this endpoint writes nothing
it reads aggregate state that contains no query text and no user data
```

A `/status` call is as privacy-neutral as fetching a favicon, and that is by design rather than
by luck: the endpoint reads index manifests, not request data.

## Degradation

| Condition | Response |
| --------- | -------- |
| Manifest unavailable | 200 with `status: "degraded"`, `index: null`, and a notice |
| Some shards down | `degraded`, with the healthy count in `verbose` |
| All shards down | `unavailable` |
| Service unhealthy but the index is fine | `healthy` for the index, `degraded` in `service` |

The index and the service are reported separately, because they fail independently and an
operator needs to know which one is the problem.

## Related

- [../operations/monitoring.md](../operations/monitoring.md) — the metrics behind these values
- [../operations/index-rebuild.md](../operations/) — how a generation is built
- [openapi.yaml](./openapi.yaml) — the contract