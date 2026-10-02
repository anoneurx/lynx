# ADR 0013: Ranking Pipeline

**Date:** 2024-04-15
**Status:** Accepted
**Deciders:** Search Team, ML Team
**Tags:** ranking, relevance, ml

## Context

Ranking requirements:
- BM25 as strong baseline
- Learning-to-rank (LambdaMART) for quality
- Fast inference (< 5ms for top-100 rerank)
- A/B testable
- Explainable (why this result?)

## Decision

**Two-stage ranking:**
1. **First-pass**: BM25 (Tantivy native) → top 1000
2. **Second-pass**: LambdaMART (LightGBM/Rust) rerank top 100 → final top 10

### BM25 (Tantivy)

```rust
// Tantivy built-in, highly optimized
let query = query_parser.parse_query(query_text)?;
let top_docs = searcher.search(&query, &TopDocs::with_limit(1000))?;
```

### LambdaMART Features

| Feature | Source |
| ------- | ------ |
| BM25 score | Tantivy |
| PageRank | Precomputed (monthly) |
| URL depth | `url.split('/').count()` |
| Domain authority | External list (Majestic/own) |
| Freshness | `now - fetched_at` |
| Click-through rate | Historical (7-day window) |
| Content length | `content.len()` |
| Language match | `query_lang == doc_lang` |

### Model Serving

- **Training**: Python (LightGBM) → export to `linfa`/`lightgbm-rs` model file
- **Inference**: Rust (`linfa-trees` or `smartcore`) → zero-copy, < 1ms for 100 docs
- **Model versioning**: Stored in S3, loaded at startup, hot-reload on `MODEL_RELOAD` signal

### Config

```toml
[ranking]
first_pass_limit = 1000
rerank_limit = 100
bm25_weight = 0.3
ltr_weight = 0.7
model_path = "/models/lambdamart_v3.bin"
```

## Consequences

### Positive
- **Best of both**: BM25 recall + LTR precision
- **Fast**: BM25 in Tantivy (C-speed), LTR only on 100 docs
- **A/B testable**: Weight config, model version, feature flags
- **Fallback**: If LTR model fails → pure BM25

### Negative
- **Two systems** to maintain (Tantivy + Rust ML)
- **Feature freshness** (CTR, PageRank) requires pipelines

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Pure BM25 | Quality ceiling; no personalization |
| Pure neural (BERT) | Too slow (100ms+ per query); overkill |
| Single-stage LTR | Need large candidate set for recall |

## Related

- ADR 0003: Tantivy (BM25)
- ADR 0014: A/B Testing Framework
- ADR 0018: Click Logging (for CTR features)