# `tests/ranking` — ranking quality and regression

Ranking correctness is a **product decision**, so its tests are golden files plus metric
gates rather than assertions of internal maths.

## Two kinds of check

**1. Golden snapshots (regression visibility)**

For a fixed judged query set and a fixed index generation, the full ranked output is
snapshotted. A PR that changes ranking produces a diff showing exactly which documents
moved, by how much, and which signal caused the movement. Reviewing that diff *is* the
review.

**2. Metric gates (correctness)**

| Metric | Gate |
| ------ | ---- |
| NDCG@10 | no drop > 5 % vs. baseline |
| Recall@50 | no drop > 5 % |
| MRR@10 | no drop > 5 % |
| Duplicate rate in top 10 | no increase |
| Spam-flagged rate in top 10 | no increase |
| p95 search latency | no increase > 20 % |
| Freshness (median age of top-10 on time-sensitive queries) | no drop |

## Fixtures

`fixtures/` contains a small, hand-written corpus (a few hundred documents) with deliberately
planted cases: exact duplicates, near-duplicates, doorway pages, keyword-stuffed pages,
fresh and stale versions of the same URL, an authoritative page with poor on-page text, and
pages that only match on a field other than body. These make signal behaviour observable
without a large corpus.

**Why a hand-written fixture rather than a crawl:** deterministic, reviewable, no
copyright concerns, and CI never touches the network.

See [docs/ranking/](../docs/ranking/README.md) and
[docs/testing/quality-evaluation.md](../docs/testing/quality-evaluation.md).