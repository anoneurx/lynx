# Research Experiments

Tracking LYNX search relevance experiments.

## Experiment Template

```markdown
# EXP-YYYY-MM-DD-<name>

**Hypothesis:** Changing X will improve Y by Z%
**Variant:** Control vs Treatment
**Metric:** NDCG@10, MRR@10, Latency p99
**Duration:** 14 days minimum
**Status:** Running / Completed / Aborted
```

## Active Experiments

| ID | Name | Hypothesis | Start | Status |
| -- | ---- | ---------- | ----- | ------ |
| EXP-2026-09-15 | `ltr_v3_model` | LambdaMART v3 (+CTR features) improves NDCG@10 by >2% | 2026-09-15 | Running |
| EXP-2026-10-01 | `bm25_k1_1.5` | BM25 k1=1.5 better for long queries | 2026-10-01 | Running |

## Completed Experiments

| ID | Name | Result | Decision |
| -- | ---- | ------ | -------- |
| EXP-2026-08-20 | `ltr_v2_model` | +1.8% NDCG@10, p=0.03 | **Launched** |
| EXP-2026-07-10 | `bm25_b_0.9` | -0.5% NDCG@10, p=0.42 | Rejected |
| EXP-2026-06-05 | `freshness_boost` | +3.2% NDCG@10 for news queries | **Launched** |
| EXP-2026-05-01 | `semantic_rerank` | +0.8% NDCG@10, +15ms latency | Rejected (latency) |

## Experiment Infrastructure

- **Assignment**: Cookie-based (ADR 0014)
- **Analysis**: `python -m lynx.eval.ab_analysis`
- **Guardrails**: Error rate, latency, zero-result rate (ADR 0014)
- **Sequential testing**: mSPRT (always-valid p-values)

## How to Launch

```bash
# 1. Create experiment config
cat > config/experiments/exp-2026-10-15-new-features.toml <<'EOF'
[exp-2026-10-15-new-features]
enabled = true
traffic_split = 0.5
groups = ["control", "treatment"]
guardrails = { error_rate = 0.01, latency_p99_ms = 300, zero_result_rate = 0.05 }
EOF

# 2. Deploy (ArgoCD syncs config)
git add config/experiments/exp-2026-10-15-new-features.toml
git commit -m "exp: add exp-2026-10-15-new-features"
git push

# 3. Monitor
# Grafana dashboard: "Experiments / exp-2026-10-15-new-features"
```