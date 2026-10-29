# `research/benchmarks/` — benchmark definitions and runners

Reproducible benchmark definitions, query sets, and runners. The in-repo quality gates
(NDCG@10, MRR@10, Recall@100, duplicate and spam ratios over the judge set) live in
`services/quality` and are documented in
[docs/testing/quality-evaluation.md](../../docs/testing/quality-evaluation.md); this
directory is for benchmarks that go beyond those gates — including any comparison
against other search engines.

No benchmark results are published from this directory yet. When they are, every number
carries the labels and the run record described below.

## Data collection and fairness

1. **Other engines' results are collected only in ways their terms of service allow.**
   Use official APIs where available (with the engine's own attribution and licence
   terms). Do not scrape engines whose terms forbid it. If a benchmark cannot be run
   within a competitor's permitted access, that engine is left out and the absence is
   stated — not worked around.
2. **Every benchmark run records, in the repository next to the results:** the date the
   data was collected, the engine and its version (or the API and API version) used, the
   query set (name, version, and provenance), the number of queries, the number of
   human raters, and whether the evaluation was blind (raters not shown which engine
   produced which result).
3. **Every benchmark number is labelled `measured` or `target`.** A `measured` number
   names the run it came from (the record above); a `target` number is a goal that has
   not been observed. This follows the "honest claims only" design principle in the
   [README](../../README.md#design-principles): targets are labelled as targets,
   limitations are documented, not hidden.
4. **Known limitations are stated with every result**, not in a footnote elsewhere:
   - **Small index.** LYNX's corpus is v0.1-scale small (the crawler, not a data
     centre, is the constraint); coverage-dependent comparisons against commercial
     engines are not like-for-like and must say so.
   - **Sample size.** Small query sets and small rater counts make numbers noisy;
     report the sample size and do not over-interpret differences within it.
   - **Rater bias.** Human judgements vary between raters and over time; report the
     rater count and whether evaluation was blind, and treat single-rater judgments as
     provisional.

## Rules

1. **A benchmark states its hypothesis and falsifiable outcome before it runs**, in a
   `README.md` next to the runner — same discipline as `research/experiments/`.
2. **Rerunnable.** Query sets, scripts, and the exact invocation are in the repository;
   a third party can reproduce a `measured` number without contacting any third-party
   service except the ones the run itself records.
3. **No query logs.** Query sets are curated or drawn from public sources with stated
   provenance — never harvested from LYNX users, consistent with the privacy model in
   [PRIVACY.md](../../PRIVACY.md).
