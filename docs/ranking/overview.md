# Ranking overview

Ranking is the part of LYNX most likely to be judged by users, and the part where a wrong
decision is hardest to explain. So it is a small number of named, bounded, documented
signals combined with one rule: quality adds, harm subtracts multiplicatively.

## Pipeline

```text
  query
    │
    ▼
  query analysis ──── operators, intent, spell correction, time sensitivity
    │
    ▼
  RETRIEVAL   BM25F over fused fields        →  candidates (≤ 2000)
    │                                           recall-oriented
    ▼
  FILTERING   freshness= / before= / after= / site= / ext= / inurl= → live filters
    │
    ▼
  NORMALISE   bounded features: lexical, freshness, quality, authority, context, spam
    │
    ▼
  AGGREGATE   Σ wᵢ·signalᵢ                                  score_raw
    │
    ▼
  PENALISE    × (1 − clamp(spam)·w_penalty)                 score_pen
    │
    ▼
  DEDUP       × cluster_penalty(cluster)                    score_final
    │
    ▼
  RERANK      reciprocal-rank fusion over topical shards (disabled by default)
    │
    ▼
  SERVE       top 10 with snippets and full signal vector
```

Retrieval and ranking are deliberately separate stages. Retrieval is tuned for recall (cheap,
broad, tolerant); ranking is tuned for precision (expensive, strict, explainable). Merging
them makes it impossible to tell whether a miss came from retrieval or from a weight.

## The signal set

Six signals. That is the whole ranking function.

| Signal | Stage | Range | Role | Detail |
| ------ | ----- | ----- | ---- | ------ |
| `lexical` | retrieval | ≥ 0 | relevance | [bm25f.md](./bm25f.md) |
| `freshness` | ranking | 0…1 | recency, context-sensitive | [freshness.md](./freshness.md) |
| `quality` | ranking | −1…1 | on-page trustworthiness | [quality.md](./quality.md) |
| `authority` | ranking | 0…1 | link-graph estimate, capped | [authority.md](./authority.md) |
| `context` | ranking | 0…1 | intent match, language, safe | this document |
| `spam` | penalty | 0…1 | harm, multiplicative | [spam.md](./spam.md) |

```text
score_raw   = lexical + 0.18·freshness + 0.12·quality + 0.15·authority + 0.10·context
score_final = score_raw × (1 − 0.40·clamp(spam,0,1)) × dedup_penalty
```

Every input is clamped before it is weighted, so a feature bug cannot produce an unbounded
score. That is the difference between a ranking system that degrades and one that breaks.

## Why `lexical` dominates

`lexical` has weight 1.00 and a scale normalised to roughly [0, 12] for a strong multi-term
match. `freshness` contributes at most 0.18. So a document that matches the query
lexically and is five years old beats a document that matches weakly and is an hour old.

This is deliberate, and it is the main reason LYNX's results feel boring-but-right: relevance
is not negotiable, and no other signal can buy its way past it.

## Weight philosophy

| Weights | Changed when |
| ------- | ------------ |
| `lexical`, `quality`, `context` | Rarely. These are the identity of the product |
| `freshness`, `authority` | When measured corpus/competition data shows a specific gap |
| `spam_penalty` | The most-tuned weight, because spam precision is the hardest measurement |

Any change bumps `ranking.config_version`, regenerates the golden set, and produces a
per-document diff reviewed before merge. A weight change without a reviewed diff does not
ship — the diff *is* the argument, and it is where unintended effects become visible.

## Context signal

The one signal defined here rather than in its own document, because it is small:

| Component | Weight | Meaning |
| --------- | ------ | ------- |
| `intent_match` | 0.40 | query class matches content type (definitional → reference, troubleshooting → docs, news → news) |
| `language_confidence` | 0.30 | declared and detected language agree with the query language |
| `safe_to_render` | 0.20 | no hostile content type, no download-only, no paywall interstitial |
| `readability_proxy` | 0.10 | structure and sentence length within a sane band |

`readability_proxy` is deliberately tiny. Readability is close to unknowable without rendering,
and a heavy readability weight would systematically favour short low-information pages.

## Reranking

Reciprocal-rank fusion over topical shards is implemented but **disabled by default**.

```text
RRF(doc) = Σ_shards  1 / (k + rank_shard(doc)),  k = 60
```

It helps when the corpus is large enough that no single shard covers a query well. It is off
because it makes the ranking story harder to explain — a result placed by fusion cannot be
justified by one weight set — and explainability outranks a small precision gain until the
fusion result is as explainable as the rest. Enabling it is a config flag plus an ADR.

## Cluster suppression

Near-duplicate results are the visible failure of a small index. After scoring:

```text
1. cluster documents by content signature (docs/indexing/dedup.md)
2. within a cluster keep the highest-scoring document
3. apply a penalty to surviving members so a site cannot occupy the whole first page
   duplicate_penalty(cluster) = 1 / (1 + 0.6·(members − 1))
```

Ten copies of the same article produce one result, not ten. The site is not suppressed
entirely — only its ability to fill a page with the same content twice.

## Evaluation

| Metric | Purpose |
| ------ | ------- |
| NDCG@10 | graded relevance, the primary metric |
| MRR@10 | how quickly a good result appears |
| Recall@50 | retrieval stage health, independent of ranking |
| Duplicate rate in top 10 | cluster suppression working |
| Spam-flagged rate in top 10 | false-positive monitoring |
| Median age of top 10 (time-sensitive subset) | freshness working |
| p95 latency | budget for signal cost |

Judged-set construction, the labelled fixtures, and the review workflow are in
[../testing/quality-evaluation.md](../testing/quality-evaluation.md).

## Known weaknesses

| Weakness | Assessment |
| -------- | ---------- |
| No learning-to-rank | Deliberate. An LTR model would learn from a click signal we refuse to collect, so there is no honest training data |
| No semantic retrieval in v0.1 | Would help paraphrase matching. Evaluated offline in `research/`, gated on explainability and latency |
| Authority is globally, not topically, computed | A domain strong in one subject looks strong in all. BM25F carries topical work |
| Static weights | No per-query-class weights. Intent detection multiplies freshness but not the others. A future refinement, requiring evidence |
| Quality signals are shallow | No rendering, so no page-speed or layout quality. Weighted low accordingly |
| Judged set is small and hand-made | At least it is honest about what was measured, and it grows |

## Testing

- Unit tests for the aggregation arithmetic: clamping, monotonicity, penalty effects.
- Property tests that hold across all signals: penalty can only reduce a score; all values
  stay within range; the order of a fixed candidate set is deterministic for a fixed
  generation and config version.
- Golden ranking snapshots with a reviewed per-document diff on every change.
- Regression tests for the specific behaviours the design promises: a current release note
  beating a five-year-old post for a "latest" query; a link-farm page losing to a natural
  equivalent; a padded thin page losing to a natural equivalent.
- Latency budget test so a new signal cannot silently push p95 past target.