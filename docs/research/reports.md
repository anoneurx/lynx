# Research Reports

Monthly and quarterly research reports.

## Report Index

| Report | Period | Focus | Link |
| ------ | ------ | ----- | ---- |
| **Q3 2026 Search Quality** | Jul–Sep 2026 | NDCG trends, experiment results | `reports/2026-q3-search-quality.md` |
| **Q2 2026 Crawl Coverage** | Apr–Jun 2026 | New domains, freshness, dedup | `reports/2026-q2-crawl-coverage.md` |
| **Q1 2026 Index Performance** | Jan–Mar 2026 | Build time, segment stats, merge | `reports/2026-q1-index-performance.md` |

## Report Template

```markdown
# LYNX Research Report — <Period>

## Executive Summary
- Key metric changes (NDCG, latency, coverage)
- Top 3 insights
- Action items

## Search Quality
### NDCG@10 Trends
![NDCG Trend](charts/ndcg_trend.png)

| Segment | Current | Previous | Δ |
| ------- | ------- | -------- | - |
| Overall | 0.658 | 0.642 | +2.5% |
| Navigational | 0.734 | 0.721 | +1.8% |
| Informational | 0.592 | 0.578 | +2.4% |

### Experiment Results
| Experiment | Lift | p-value | Decision |
| ---------- | ---- | ------- | -------- |
| exp-2026-09-ltr_v3 | +2.3% | 0.003 | **Launched** |
| exp-2026-08-bm25_k1 | -0.5% | 0.42 | Rejected |

## Crawl Health
- New domains crawled: 12,400
- Pages/day: 2.1M (avg)
- Freshness (top-10 median age): 14 days
- Dedup rate: 18% (exact), 5% (near)

## Index Performance
- Generation build time: 1.8 hr (100M docs)
- Segment count: 47 (target < 50)
- Merge overhead: 12% of build time
- Reader memory: 3.2 GiB

## Infrastructure
- API p99 latency: 138 ms (target < 200 ms)
- Crawler queue depth: < 5k (healthy)
- Error budget remaining: 87%

## Risks & Mitigations
| Risk | Likelihood | Impact | Mitigation |
| ---- | ---------- | ------ | ---------- |
| Index build > 24h | Medium | High | Sharding (ADR 0012) |
| Query latency regression | Low | High | Canary + SLO alerts |
| Crawl politeness violations | Low | Medium | Automated monitoring |

## Appendix
- Raw data: `eval/reports/2026-q3/`
- Notebooks: `notebooks/2026-q3-analysis.ipynb`
```

## Automated Generation

```bash
# Monthly (1st of month, 06:00 UTC)
# Generates from eval/ data + GitHub issues + Grafana snapshots
python -m lynx.eval.generate_report \
  --period monthly \
  --output docs/research/reports/$(date +%Y-%m)-report.md
```

## Distribution

- **Internal**: Posted to `#lynx-research` Slack, GitHub Discussions
- **Stakeholders**: Email to product, eng leads
- **Archive**: `docs/research/reports/` (Git history)