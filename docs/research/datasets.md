# Datasets

Curated datasets for training, evaluation, and benchmarking.

## Judge Sets (Human-Labeled)

| File | Queries | Labels/Query | Annotators | Created | Use Case |
| ---- | ------- | ------------ | ---------- | ------- | -------- |
| `v2026-q1.jsonl` | 2,000 | 10 | 5 | 2026-03-15 | Baseline |
| `v2026-q2.jsonl` | 2,500 | 10 | 5 | 2026-06-20 | Seasonal |
| `v2026-q3.jsonl` | 3,000 | 15 | 5 | 2026-09-10 | **Current production** |
| `v2026-q4.jsonl` | 3,000 | 15 | 5 | 2026-12-01 | Staging |

### Format (JSONL)

```json
{"query": "rust async runtime", "judgments": [{"doc_id": "a1b2c3d4", "relevance": 3}, {"doc_id": "e5f6g7h8", "relevance": 1}]}
```

- `relevance`: 0 (irrelevant) – 3 (perfect)
- `doc_id`: BLAKE3 hash of URL (first 8 bytes)

### Agreement

```bash
# Compute Fleiss' kappa
python -m lynx.eval.agreement --judge-set eval/judge-sets/v2026-q3.jsonl
# κ = 0.68 (substantial agreement)
```

## Synthetic Query Sets

| File | Size | Generation Method | Use Case |
| ---- | ---- | ----------------- | -------- |
| `synthetic/queries_10k.txt` | 10,000 | LLM templates | Coverage testing |
| `synthetic/queries_100k.txt` | 100,000 | LLM templates | Load testing |
| `synthetic/adversarial.txt` | 1,000 | Hand-crafted | Robustness |

### Generation

```bash
# Generate new synthetic queries
python -m lynx.eval.synthetic.generate \
  --templates eval/synthetic/templates.yaml \
  --count 50000 \
  --output eval/synthetic/queries_50k.txt
```

## Click Logs (Production)

- **Source**: Search API click tracking (opt-in, anonymized)
- **Schema**: `query_hash`, `doc_id`, `position`, `dwell_time`, `timestamp`
- **Retention**: 90 days (GDPR)
- **Access**: Internal only, aggregated for CTR features

```bash
# Export for LTR training (weekly)
python -m lynx.eval.export_clicks \
  --since 7d \
  --output eval/ltr/clicks_weekly.parquet
```

## Document Corpus Samples

| File | Size | Source | License |
| ---- | ---- | ------ | ------- |
| `corpus/sample_10k.jsonl` | 10,000 | Common Crawl (CC-BY) | CC-BY 4.0 |
| `corpus/sample_100k.jsonl` | 100,000 | Common Crawl (CC-BY) | CC-BY 4.0 |

### Format

```json
{"url": "https://example.com/page", "title": "Example Page", "text": "Page content...", "lang": "en", "fetched_at": "2026-09-15T10:30:00Z"}
```

## Downloading

```bash
# Judge sets (internal)
aws s3 sync s3://lynx-research/judge-sets/ eval/judge-sets/

# Public corpora
aws s3 sync s3://lynx-public/corpus/ eval/corpus/ --no-sign-request
```

## Dataset Cards

Each dataset has a `DATASET_CARD.md` with:
- Intended use
- Collection methodology
- Bias/limitations
- Preprocessing
- Licensing