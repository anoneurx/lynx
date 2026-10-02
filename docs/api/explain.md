# `GET /api/v1/explain`

Why is this ranked here. Operator-only in v0.1, because `verbose=true` can include query terms.

## Request

```bash
curl -sG 'https://lynx.example/api/v1/explain' \
  -H "Authorization: Bearer $LYNX_OPERATOR_TOKEN" \
  --data-urlencode 'q=rust async runtime' \
  --data 'candidate_limit=20'
```

| Parameter | Type | Default | Notes |
| --------- | ---- | ------- | ----- |
| `q` | string | required | Same grammar as `/search` |
| `candidate_limit` | int | 20 | 1–200. How far into the ranking to explain |
| `include_shards` | bool | false | Per-shard retrieval contributions |
| `include_cluster` | bool | true | Cluster membership and penalty |
| `include_spam` | bool | true | Spam signals that fired |
| `include_banned` | bool | false | Include results suppressed by policy |

## Response

```json
{
  "query_plan": {
    "raw_length": 17,
    "must": ["rust", "async", "runtime"],
    "should": [],
    "must_not": [],
    "phrases": [],
    "disjunction": false,
    "operators_used": [],
    "intent": "informational",
    "language": { "tag": "en", "confidence": 0.96 },
    "time_sensitive": "none",
    "corrections": [],
    "expansions": []
  },
  "index_generation": "gen-2026-02-17T04:00:00Z",
  "config_version": "v0.1.0",
  "results": [
    {
      "rank": 1,
      "doc_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV",
      "url": "https://tokio.rs/",
      "score": 12.4817,
      "lexical": { "value": 8.4213, "weight": 1.0, "contribution": 8.4213,
                   "by_field": { "title": 5.10, "headings": 1.88,
                                 "body": 1.20, "url": 0.24, "anchors": 0.02 } },
      "signals": [
        { "name": "freshness", "value": 0.83, "weight": 0.18, "contribution": 0.1494 },
        { "name": "quality",   "value": 0.61, "weight": 0.12, "contribution": 0.0732 },
        { "name": "authority", "value": 0.51, "weight": 0.15, "contribution": 0.0765 },
        { "name": "context",   "value": 1.00, "weight": 0.10, "contribution": 0.1000 }
      ],
      "score_raw": 8.8204,
      "spam": { "score": 0.0, "signals_fired": [] },
      "spam_penalty": 1.0,
      "cluster": { "id": null, "penalty": 1.0, "member_count": 1 },
      "duplicate_penalty": 1.0,
      "score_final": 8.8204
    }
  ],
  "normalisation": {
    "candidates_retrieved": 1841,
    "candidates_ranked": 1841,
    "suppressed_by_dedup": 12,
    "suppressed_by_policy": 3,
    "shards_queried": 2,
    "shards_timed_out": 0
  },
  "timing_ms": { "parse": 1, "retrieve": 24, "filter": 1, "rank": 4, "snippet": 6, "total": 41 }
}
```

## Reading it

```text
score_raw   = lexical + Σ(signal.value × signal.weight)
             = 8.4213 + 0.1494 + 0.0732 + 0.0765 + 0.1000
             = 8.8204

score_final = score_raw × spam_penalty × duplicate_penalty
             = 8.8204 × 1.0 × 1.0
             = 8.8204
```

Every number is derivable from the response. That is the property the endpoint exists to
guarantee: an operator can check the arithmetic and find where the ranking disagrees with their
expectation.

The `lexical.by_field` breakdown is usually the fastest diagnostic. If `body` dominates on a
result that should be a title match, the field weights are the thing to look at
([../ranking/bm25f.md](../ranking/bm25f.md)); if `title` is zero, the title index is not being
populated.

## Authentication

```text
Authorization: Bearer <operator token>
scopes         explain, admin, keys
```

| Token | Access |
| ----- | ------ |
| None | 401 |
| Operator, `explain` scope | Full explanations |
| Operator, `keys` scope only | 403, scope mismatch |
| Expired or revoked | 401 |
| From an untrusted network | Rejected by the edge before the handler |

The route is never internet-exposed even with a valid token. It runs behind the same VPN path as
`/admin/*`, because it can expose query terms an operator should not need to see incidentally.

## Reproducibility

```text
score(doc) = f(doc_id, index_generation, ranking_config_version, query)
```

All four are in the response, so an explanation remains valid after the fact:

```bash
lynx-query score --doc <doc_id> --generation <gen> --config <version> --query '<q>'
```

An explanation can therefore be re-derived months later, which is what makes a ranking complaint
answerable rather than merely arguable.

## Cost and caching

```text
Cache-Control: no-store
```

Not because anything is sensitive that is not already sensitive, but because an explanation is
operator output and intermediaries should not retain it. The handler is otherwise the same
pipeline as `/search` plus the breakdown, so its cost is close to a search.

## What it cannot explain

| Limit | Honest statement |
| ----- | ---------------- |
| Why the corpus contains it | Explain covers ranking, not coverage |
| PageRank's derivation | It is a property of the graph. The value and its computation date are reported, not its derivation per link |
| Interaction between penalties | A multiplicative penalty appears as its factor, but the joint effect is not further decomposed |
| Why a user saw a different order | The same query against a different generation gives a different order, and `index_generation` is the discriminator |

That last row is worth internalising: a complaint about ranking is often a complaint about index
age. The generation in the explanation is usually the answer.

## Related

- [../ranking/explainability.md](../ranking/explainability.md) — the commitment this implements
- [../ranking/](../ranking/) — the signals being explained
- [search.md](./search.md) — the endpoint it mirrors
- [../security/key-management.md](../security/key-management.md) — operator tokens