# Search Quality Evaluation

## Metrics

| Metric | Formula | Target |
| ------ | ------- | ------ |
| **NDCG@10** | $\frac{DCG@10}{IDCG@10}$ | ≥ 0.65 |
| **MRR@10** | $\frac{1}{|Q|}\sum \frac{1}{rank_i}$ | ≥ 0.55 |
| **Recall@100** | $\frac{|relevant \cap retrieved|}{|relevant|}$ | ≥ 0.80 |
| **Latency p99** | — | < 200 ms |

## Judge sets

Curated query → relevance labels (0–3).

```
eval/judge-sets/
  ├── v2026-q1.jsonl    # 2,000 queries, 5 annotators each
  ├── v2026-q2.jsonl
  ├── v2026-q3.jsonl    # current production
  └── v2026-q4.jsonl    # staging
```

Format (JSONL):

```json
{"query": "rust async runtime", "judgments": [{"doc_id": "hash1", "relevance": 3}, {"doc_id": "hash2", "relevance": 1}]}
```

## Evaluation CLI

```bash
# Run against current production index
python -m lynx.eval.ndcg \
  --judge-set eval/judge-sets/v2026-q3.jsonl \
  --endpoint https://api.lynx.example.com \
  --top-k 10 \
  --output eval/runs/2026-10-02_prod.json

# Compare two runs
python -m lynx.eval.compare \
  --baseline eval/runs/2026-09-15_prod.json \
  --candidate eval/runs/2026-10-02_prod.json \
  --metric ndcg@10
```

Output:

```
NDCG@10:  baseline=0.642  candidate=0.658  Δ=+2.5%  p=0.003 ✅
MRR@10:   baseline=0.541  candidate=0.552  Δ=+2.0%  p=0.012 ✅
Recall@100: baseline=0.78  candidate=0.81    Δ=+3.8%  p=0.001 ✅
Latency p99: baseline=142ms  candidate=138ms  Δ=-2.8%  ✅
```

## Regression detection (CI)

```yaml
# .github/workflows/quality-eval.yml
on:
  schedule:
    - cron: '0 4 * * 1'  # Weekly Monday
  workflow_dispatch:
    inputs:
      judge_set:
        description: 'Judge set version'
        required: true
        default: 'v2026-q3'

jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: '3.12' }
      - name: Install lynx-eval
        run: pip install -e ./eval
      - name: Run evaluation
        id: eval
        run: |
          python -m lynx.eval.ndcg \
            --judge-set eval/judge-sets/${{ github.event.inputs.judge_set }}.jsonl \
            --endpoint https://api.lynx.example.com \
            --output run.json
      - name: Check regression
        run: |
          python -m lynx.eval.regression \
            --current run.json \
            --baseline eval/baselines/${{ github.event.inputs.judge_set }}.json \
            --threshold 0.01  # 1% relative drop
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

## A/B testing

### Framework

- **Assignment:** Cookie-based (`lynx_exp=control|treatment`), 50/50 split.
- **Duration:** Minimum 14 days, max 28 days.
- **Guardrails:** Error rate, latency, zero-result rate.

### Implementation

```rust
// lynx-api/src/experiment.rs
pub fn assign_experiment(req: &HttpRequest) -> ExperimentGroup {
    let cookie = req.cookie("lynx_exp").map(|c| c.value());
    match cookie {
        Some("treatment") => ExperimentGroup::Treatment,
        Some("control") => ExperimentGroup::Control,
        _ => {
            let group = if fastrand::u8(..) < 128 { Treatment } else { Control };
            // Set cookie for session
            group
        }
    }
}
```

### Analysis

```python
# eval/ab_analysis.py
def analyze(experiment_id: str):
    control = load_metrics(f"{experiment_id}_control")
    treatment = load_metrics(f"{experiment_id}_treatment")

    # Sequential testing (always valid p-values)
    from scipy.stats import mannwhitneyu
    stat, p = mannwhitneyu(treatment.ndcg_scores, control.ndcg_scores, alternative='greater')

    # Practical significance
    lift = (treatment.ndcg_mean - control.ndcg_mean) / control.ndcg_mean

    return {
        "p_value": p,
        "lift": lift,
        "significant": p < 0.05 and lift > 0.01,
        "sample_size": len(treatment.ndcg_scores),
    }
```

## Click-through modeling (offline)

Train **LambdaMART** on historical clicks:

```python
# eval/ltr/train.py
from sklearn.datasets import load_svmlight_file
from pyltr import LambdaMART

X, y, qids = load_svmlight_file("eval/ltr/train.txt", query_id=True)
model = LambdaMART(n_estimators=500, learning_rate=0.05, max_features=0.5)
model.fit(X, y, qids)

# Export as Tantivy custom scoring function
model.save("models/lambdamart_v3.pkl")
```

Deploy via config:

```toml
# config.toml
[ranking]
model = "lambdamart_v3"
bm25_weight = 0.3
ltr_weight = 0.7
```

## Synthetic query generation

For coverage testing:

```python
# eval/synthetic/generate.py
import llm  # local LLM

templates = [
    "how to {verb} {noun} in {language}",
    "best {noun} for {use_case}",
    "{error_message} fix",
]

queries = []
for _ in range(10_000):
    t = random.choice(templates)
    q = llm.complete(f"Fill: {t}").text
    queries.append(q)

save("eval/synthetic/queries_10k.txt", queries)
```

Run against index, flag zero-result queries for crawl prioritization.

## Data quality checks

| Check | Tool | Threshold |
| ----- | ---- | --------- |
| Judge set agreement (Fleiss' κ) | `eval/agreement.py` | κ > 0.6 |
| Query distribution drift | KS-test on term freq | p > 0.05 |
| Index coverage | `% queries with ≥1 result` | > 99.5 % |
| Freshness | `max(doc_age) for top-10` | < 30 days |

## Reporting

Monthly **Search Quality Report** (auto-generated):

```
# Search Quality Report — 2026-10

## Headline metrics
NDCG@10: 0.658 (+0.016 vs last month)
MRR@10:  0.552 (+0.011)
Zero-result rate: 0.8% (-0.2%)

## Top regressions (if any)
- "how to center div" NDCG dropped 0.12 → CSS spec changed
- "python async await" MRR dropped 0.08 → new tutorials not crawled

## Crawl gaps identified
- 1,200 queries with zero results → 340 new seed URLs added

## A/B experiments
- exp-2026-09-ltr: +2.3% NDCG, LAUNCHED
- exp-2026-09-bm25-k1: -0.5% NDCG, REJECTED
```

## Baseline management

- Baselines stored in `eval/baselines/<judge_set>.json`.
- Updated **only** when:
  1. New judge set released (quarterly).
  2. A/B experiment launches (new baseline = treatment).
  3. Major index rebuild (generation +10%).

Never update baseline to hide regression.