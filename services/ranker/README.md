# `services/ranker` — ranking engine

**Deployment:** v0.1 — library inside `apps/api`. Runs on the candidate set returned by
the index reader. Full design: [docs/ranking/](../../docs/ranking/README.md).

## Contract

```text
rank(candidates, query_features, ranking_config) -> Vec<RankedResult>

RankedResult {
  doc_id, url, title, snippet, score,
  signals: { bm25, freshness, quality, authority, spam, … },   ← always computed
  config_version, index_version                                 ← reproducibility
}
```

## Principles

1. **Transparent.** Every score is the sum of named, weighted, bounded signals. There is no
   learned model in the critical path in v0.1, and no signal is undocumented.
2. **Reproducible.** `(doc_id, index_version, ranking_config_version)` determines the score.
   We can reproduce any past ranking from these three values, which is what makes quality
   regressions debuggable rather than mysterious.
3. **Explainable.** The signal vector is retained for every served result (in memory, with
   a short TTL cache) so a "why is this ranked here" question can be answered without
   re-running the pipeline. Exposed to operators via the admin console and to users via a
   `/api/v1/explain` endpoint on a whitelisted path.
4. **Extensible.** Signals are registered features with a name, a range, a default weight,
   and a validity predicate. Adding a signal is a config change plus a function, not an
   architecture change.

## Signal families

| Family | Examples | Range |
| ------ | -------- | ----- |
| Lexical | BM25F per field, phrase proximity, coverage | 0..∞ (normalised) |
| Freshness | crawl recency, publication recency decay | 0..1 |
| Quality | title present, canonical, https, meta description, content depth | −1..1 |
| Authority | PageRank, domain authority (capped), external link ratio | 0..1 |
| Spam | stuffing, doorway cluster, link-farm features, cloaking | −1..0 (penalty) |
| Context | exact-domain match, language match, `site:` intent, freshness intent | 0..1 |

Aggregation is a weighted linear combination with multiplicative penalties, followed by
near-duplicate cluster suppression. The exact formula, the weights, and the rationale for
each weight live in [docs/ranking/](../../docs/ranking/README.md) and are versioned in
`config/`.

## Anti-goal

The ranker must not become a black box. If a future change replaces the linear combination
with a learned model, that is a new ADR, must ship behind a shadow-scoring mode, and must
retain the ability to explain any individual result.