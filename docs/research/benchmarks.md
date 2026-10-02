# Benchmarks

Standardized benchmarks for LYNX search quality and performance.

## Search Quality Benchmarks

### Internal (Judge Sets)

| Benchmark | Judge Set | Metric | Baseline (v2026-q3) | Target |
| --------- | --------- | ------ | ------------------- | ------ |
| **Overall** | v2026-q3 | NDCG@10 | 0.642 | ≥ 0.65 |
| **Overall** | v2026-q3 | MRR@10 | 0.541 | ≥ 0.55 |
| **Overall** | v2026-q3 | Recall@100 | 0.78 | ≥ 0.80 |
| **Navigational** | v2026-q3 (subset) | NDCG@10 | 0.72 | ≥ 0.75 |
| **Informational** | v2026-q3 (subset) | NDCG@10 | 0.58 | ≥ 0.60 |
| **Long-tail** | v2026-q3 (subset) | NDCG@10 | 0.51 | ≥ 0.55 |
| **Non-English** | v2026-q3 (subset) | NDCG@10 | 0.49 | ≥ 0.52 |

Run:
```bash
python -m lynx.eval.ndcg --judge-set eval/judge-sets/v2026-q3.jsonl --breakdown
```

### External (BEIR)

| Dataset | Domain | NDCG@10 (BM25) | NDCG@10 (LYNX) | Delta |
| ------- | ------ | -------------- | -------------- | ----- |
| MS MARCO | Passage retrieval | 0.187 | 0.195 | +4.3% |
| TREC-COVID | Biomedical | 0.654 | 0.671 | +2.6% |
| NFCorpus | Medical | 0.325 | 0.338 | +4.0% |
| HotpotQA | Multi-hop QA | 0.412 | 0.425 | +3.2% |
| FiQA | Financial | 0.298 | 0.311 | +4.4% |
| ArguAna | Argument retrieval | 0.315 | 0.328 | +4.1% |
| **Average** | — | **0.365** | **0.378** | **+3.6%** |

Run:
```bash
python -m lynx.eval.beir --datasets all --output eval/benchmarks/beir_$(date +%F).json
```

## Performance Benchmarks

### API Latency (Local, 10M docs)

| Percentile | Baseline | Target |
| ---------- | -------- | ------ |
| p50 | 28 ms | ≤ 30 ms |
| p90 | 65 ms | ≤ 70 ms |
| p99 | 142 ms | ≤ 150 ms |
| p999 | 280 ms | ≤ 300 ms |

```bash
# Run with vegeta
vegeta attack -rate=1000 -duration=60s -targets=targets.txt | vegeta report
```

### Index Build

| Corpus Size | Machine | Build Time | Peak Memory |
| ----------- | ------- | ---------- | ----------- |
| 10M docs | 8 vCPU, 32 GiB, NVMe | 12 min | 8 GiB |
| 100M docs | 16 vCPU, 64 GiB, NVMe | 2.1 hr | 16 GiB |
| 1B docs | 32 vCPU, 128 GiB, NVMe | 22 hr | 32 GiB |

```bash
# Benchmark indexer
cargo run --bin lynx-indexer -- --benchmark --docs 1000000
```

### Crawler Throughput

| Workers | Pages/min | CPU/util | Network |
| ------- | --------- | -------- | ------- |
| 10 | 15,000 | 40% | 50 Mbps |
| 50 | 70,000 | 65% | 200 Mbps |
| 200 | 250,000 | 85% | 800 Mbps |

## Hardware Baselines

### Search API (per pod)

| Config | QPS | p99 Latency | Memory |
| ------ | --- | ----------- | ------ |
| 2 vCPU, 4 GiB | 800 | 120 ms | 1.2 GiB |
| 4 vCPU, 8 GiB | 1,800 | 95 ms | 2.1 GiB |
| 8 vCPU, 16 GiB | 3,500 | 85 ms | 3.8 GiB |

### Indexer

| Config | Docs/sec | Build Time (100M) | Memory |
| ------ | -------- | ----------------- | ------ |
| 8 vCPU, 32 GiB | 12,000 | 2.3 hr | 8 GiB |
| 16 vCPU, 64 GiB | 25,000 | 1.1 hr | 16 GiB |
| 32 vCPU, 128 GiB | 48,000 | 0.6 hr | 32 GiB |

## Regression Detection

CI fails if:
- NDCG@10 drops > 1% vs baseline
- p99 latency increases > 10% vs baseline
- Build time increases > 15% vs baseline

```bash
# In CI
python -m lynx.eval.regression \
  --current eval/benchmarks/current.json \
  --baseline eval/benchmarks/baseline_v2026-q3.json \
  --threshold 0.01
```