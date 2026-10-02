# Explainability

If a user asks "why is this ranked here", LYNX can answer exactly. That capability is a
design requirement, not a feature, and it constrains what ranking is allowed to become.

## The contract

```rust
struct RankedResult {
    doc_id, url, title, snippet,
    score: f64,
    signals: SignalVector,          // every component, always computed
    ranking_config_version: String,
    index_generation: String,
}

struct SignalVector {
    lexical: f64,
    lexical_by_field: BTreeMap<Field, f64>,
    bm25f: f64,
    freshness: f64,
    quality: f64,
    authority: f64,
    context: f64,
    spam_score: f64,
    spam_signals_fired: Vec<&'static str>,
    duplicate_penalty: f64,
    cluster_id: Option<Uuid>,
    query_term_coverage: f64,
}
```

Every served result carries its full signal vector. This is cheap (all inputs are already
stored fast fields, so no extra I/O) and it is what makes the system debuggable.

## Reproducibility

```text
score(doc) = f(doc_id, index_generation, ranking_config_version, query)
```

All four values are stored or returned with the result. Given them, anyone can recompute the
exact score. Consequences:

- A ranking complaint becomes answerable days later, not just in the moment.
- A ranking regression can be reproduced from a report, not reconstructed from memory.
- Two runs of the same query against the same generation are identical, which is what makes
  the CI gates meaningful.
- A quality claim in a document is verifiable, because the exact inputs are known.

## Interfaces

### `/api/v1/explain` (operator-only in v0.1)

```json
{
  "query_plan": { "must": 2, "should": 1, "must_not": 0, "filters": 1,
                  "operators_used": ["site"] },
  "config_version": "v0.1.0",
  "index_generation": "gen-2026-02-17T04:00:00Z",
  "results": [
    {
      "doc_id": "01JX8QK2M9F4T7YB3W5N6P8R0SV",
      "score": 12.4817,
      "contributions": [
        { "signal": "lexical",  "value": 8.42, "weight": 1.00, "contribution": 8.42 },
        { "signal": "authority","value": 0.51, "weight": 0.15, "contribution": 0.077 },
        { "signal": "freshness","value": 0.83, "weight": 0.18, "contribution": 0.149 },
        { "signal": "quality",  "value": 0.61, "weight": 0.12, "contribution": 0.073 },
        { "signal": "context",  "value": 1.00, "weight": 0.10, "contribution": 0.100 }
      ],
      "score_raw":       8.819,
      "spam_penalty":    1.00,
      "duplicate_penalty": 1.00,
      "score_final":     8.819,
      "rank": 3
    }
  ],
  "normalisation": { "candidates_retrieved": 1841, "candidates_ranked": 1841,
                     "suppressed_by_dedup": 12 }
}
```

Operator-only because `query_plan` in verbose form can contain query terms. For an ordinary
query, `/search?explain=1` returns the signal values but not the compiled plan's terms — the
user learns how the ranking worked without LYNX retaining what they typed.

### The admin console

Every result set an operator views carries the signal vector, expandable per document:
which signals fired, by how much, and which spam signals were present. Ranking complaints
start here.

### Version reports

Each monthly quality report includes, per query class, the distribution of signal
contributions. A change in how much freshness contributes across the corpus is visible
before anyone notices that results feel wrong.

## Rules that keep this honest

1. **Every signal is documented** in `docs/ranking/` with its range, its computation, and
   its known weaknesses.
2. **Every weight is config**, versioned, and attached to results.
3. **Clamping is enforced in code**, so no signal can escape its declared range.
4. **Spam fires are named**, not just a score. "Why is this down?" → "thin_content".
5. **Golden diffs are reviewed per document**, so a ranking change is a list of movements with
   causes, not a number that moved.
6. **The explain path has no cost when unused** — signals are computed during normal ranking,
   so answering the question never requires a special mode that might behave differently.

## Limits

| Limit | Statement |
| ----- | --------- |
| Explain is per document, not per query path | "Why is *this* ranked here", not "why did the query match this way" |
| Contributions are additive, so interaction is implicit | A multiplicative penalty appears as its own factor, which is visible, but the interaction between lexical and spam is not decomposed further |
| PageRank cannot be explained per document | It is a property of the graph, not of the page. LYNX reports the value, the top contributing inbound links, and the computation date — which is the honest limit of link-graph authority |
| Explain does not justify the corpus | A well-ranked page on a topic nobody has crawled is well-ranked within what we have |
| No A/B test of user outcomes | We cannot measure which ranking users prefer without tracking them, so ranking improvements are argued from judged metrics and stated uncertainty |

That last row is the honest position on the whole system. LYNX cannot run the experiment a
commercial engine runs constantly — does this change make people click more? — because the
answer requires knowing who searched what. LYNX substitutes judged metrics and publishes the
substitution rather than pretending the experiment happened.

## Testing

- **Determinism:** the same query against the same generation produces byte-identical
  explain output. A test enforces it.
- **Completeness:** every signal in the enumeration appears in the vector for every result —
  a missing signal is a test failure, not a silent omission.
- **Arithmetic:** `score_final == score_raw × spam_penalty × duplicate_penalty` within
  floating-point tolerance, asserted for every result.
- **Version attribution:** every result carries a config version and a generation that match
  the index manifest.
- **Regression:** an explain endpoint change that drops a field fails the API contract test.