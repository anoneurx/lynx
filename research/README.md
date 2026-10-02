# `research/` — the research component

Not everything in LYNX is engineering. This directory separates **what we build** from
**what we are trying to find out**, so that experiments have a home that is not the
production code path.

| Directory | Contents |
| --------- | -------- |
| [`experiments/`](./experiments/) | Isolated experiments, one directory each, each with a `README.md` stating the hypothesis, method, and result |
| [`datasets/`](./datasets/) | Derived datasets with provenance, licence, and a generation script. No unlicensed third-party corpora |
| [`benchmarks/`](./benchmarks/) | Reproducible benchmark definitions and runners |
| [`papers/`](./papers/) | Annotated bibliography — our notes on external work, with citations |
| [`reports/`](./reports/) | Written findings, quarterly quality reports, post-mortems of ranking changes |

## Rules

1. **Experiments never run in production.** An experiment is a separate crate or script
   with its own dependencies. Production binaries do not gain experimental feature flags
   to support a paper.
2. **Every experiment states a hypothesis and a falsifiable outcome** before it runs, and
   records a negative result just as prominently as a positive one.
3. **No experiment may require logging user queries.** Any evaluation that would require
   real query data is off-limits by policy. Where online evaluation would normally use
   click logs, LYNX uses explicit opt-in research mode with documented consent
   ([docs/testing/quality-evaluation.md](../docs/testing/quality-evaluation.md)), or
   offline evaluation over a curated query set.
4. **Results must be reproducible.** Each experiment pins its dataset version, config, and
   index generation, and can be re-run from a clean checkout.
5. **A research result that changes production requires an ADR.** The experiment stays
   here; the decision gets recorded with its trade-offs.

## Research agenda

Priority order, with the question each must answer:

| Area | Question | Why it matters |
| ---- | -------- | -------------- |
| Ranking | Which signal combination maximises NDCG@10 on our judged set without hurting latency? | This is the core quality lever |
| Ranking | Is a learned reranker worth it at our scale, and can it stay explainable? | Only after the transparent baseline is measured |
| Spam detection | Can we detect doorway/cloaking clusters with features that do not rely on a black-box classifier? | Spam is the main quality risk as we crawl more |
| Privacy-preserving search | Can we support abuse mitigation and quality evaluation without logging queries? | Already the hardest constraint we have chosen |
| Distributed indexing | At what document count does single-node Tantivy stop meeting our latency target? | Determines when Phase 6 starts |
| Semantic retrieval | Does embedding-based retrieval beat BM25F on our judged set at acceptable cost? | Longer term; must be evaluated honestly |
| Query understanding | Which corrections and expansions actually improve metrics rather than adding latency? | Cheap wins, measurable |
| AI search | Does a grounded answer layer improve task completion over a SERP, and what does it cost? | Only after retrieval is solid |
| Search quality evaluation | What is the minimum viable judged set to detect regressions reliably? | Without this, nothing else can be validated |

Current status: nothing started. Phase 3 delivers the first entries.