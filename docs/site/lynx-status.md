# LYNX — project status for the web page

> Source page: `http://localhost:5173/en-us/opensource/lynx`
> This file holds the current, copy-pasteable status of the LYNX project,
> including the structural gaps that were surveyed in existing search engines
> and which are covered in the v0.1.0 release. Update this file first, then
> mirror it to the web page.

## LYNX — status

**v0.1.0 released and published** — 2026-10-27.

Live release: https://github.com/anoneurx/lynx/releases/tag/v0.1.0
Repository: https://github.com/anoneurx/lynx

LYNX is a privacy-structural search engine: it crawls public web pages under
`robots.txt`, indexes what it is allowed to index, and serves results without
building a profile of who asked what. No query logs, no cookies, no
fingerprinting, no third-party assets — privacy is enforced by the design of
the system, not by a settings page. Ranking is explainable by construction
(versioned BM25F signal weights), and the optional "Astra" answer layer
refuses to state anything it cannot ground in a verified citation.

### What v0.1.0 contains

- **Crawler** — robots.txt compliance, per-host politeness, daily budgets,
  crawl-trap detection, full SSRF/DNS-pinning fetch safety gate
- **Indexer** — content-addressed documents, dedup clusters, sealed
  immutable generation manifests; the public API refuses to boot in
  production without one (no "accidentally empty" search surfaces)
- **Query processor** — operators: `site:`, `intitle:`, `inanchor:`, `-`
  exclusions, `"phrase"`, `after:`/`before:`, `OR` with count operators,
  plus corpus-derived fuzzy suggestions
- **Ranker** — explainable BM25F: field weights, freshness, quality,
  authority, negation and duplicate penalties, all computed at query time
- **Public API** — JSON and server-rendered HTML SERP (works with
  JavaScript off), keyset pagination, rate limiting, security headers,
  typed errors, `no-store` across every response, `no-referrer` policy
- **Astra answer layer (optional, off by default)** — separate process,
  sentence-level citation verification, typed refusals, kill switch,
  token budget, circuit breaker, answer cache, source-trust gate
- **Quality harness** — judged-set golden gates (NDCG, MRR, recall) hold
  on every test run; 56 test suites green at every gated commit

### Measured on the release build (small-corpus rehearsal)

| Surface | p50 | p90 | p99 | Burst throughput |
| --- | --- | --- | --- | --- |
| Fixture index | 0.57 ms | 1.05 ms | 1.45 ms | 1,818 req/s |
| Sealed Tantivy generation | 1.71 ms | 3.60 ms | 4.48 ms | 1,822 req/s |

Quality gates: NDCG@10 0.966, MRR@10 1.000, Recall@100 0.889 on the v0
judged set. Conditions: local machine, release binary, small corpus —
these are the serving planes' fixed overhead, not scale claims.

### Honest not-yet list (by design, not hidden)

- Corpus is prototype-scale; Phase 6 (sharding, distributed crawl,
  reader replication, multi-region DR) is future work with exit criteria
- Sentence-level verification is content-blind in v0.1 (no NLI pass yet)
- Budget, breaker, and answer cache are process-local (single replica)

---

## The gaps we surveyed in other engines — and covered

Survey method: public documentation, outage history, journalism on AI
answer citations, and published engine self-descriptions. The three gaps
below are *structural* in the rest of the landscape; each row states what
other engines do and what LYNX does instead.

### Gap 1 — the result list itself is rented

Almost every privacy-oriented or general-purpose alternative serves its
organic web links out of someone else's index. DuckDuckGo, Startpage,
Ecosia, Qwant, Ekoru and friends syndicate Bing and/or Google under
contract: your choices upstream can be privacy-respecting while the
ranking you see is still produced by the company you decided to leave.
The dependency is load-bearing — a 2024 Bing API outage left several
"private" engines returning no web results at all. Only Brave and Mojeek
operate genuinely independent indexes.

**Covered in LYNX:**

- Crawl → index → rank → serve is a single fully owned pipeline; no
  syndication contract anywhere in the loop.
- robots.txt-completely-respecting crawler with its own frontier,
  budget, and politeness layer.
- Own index generations, sealed and content-addressed, with a
  reproducible build and a documented upgrade path.
- Privacy is structural (no user data exists to protect), and the results
  are LYNX's own — the independence claim and the privacy claim do not
  contradict each other.

### Gap 2 — AI answers bloom where you cannot verify them

Independent evaluations keep confirming the same failure: the CJR Tow
Center (2025) tested eight AI search tools and found citation problems in
all eight — for some, the majority of citations led to fabricated or
error-page URLs. Research also shows a citation marker *increases* user
trust even when the citation is wrong. Meanwhile hallucinated citations
are rising measurably in both academic literature and product search
(AI overviews, chat-with-your-engine surfaces).

**Covered in LYNX (Astra, optional, off by default):**

- Fail-closed by construction: every sentence must carry a verified `[n]`
  citation to a real grounding document; sentences without one are
  stripped, and a summary that loses more than half its sentences is
  discarded entirely.
- No verified citations → **no answer**, not a guess. Refusals are typed
  (`killed`, `budget_exhausted`, `circuit_open`, `no_sources`,
  `unreliable_summary`, `empty_output`, `provider_error`). Absence in the
  SERP is the honest, normal state — there is no `answered:false` surface
  to mislead anyone.
- Source-trust gate (`ai.min_source_trust`): below-trust pages are
  withheld from the grounding head per candidate. A spammy page may rank
  (ranking is relevance) but is never handed to the model as truth.
- The model runs in a sandboxed answer plane with a two-field prompt, a
  data containment channel, no tools, and a prompt-injection detector
  over Unicode-normalised input.
- Honest limit stated next to the claim: v0.1 verification is sentence-
  level and content-blind — it bounds blast radius; it does not yet prove
  truth (the NLI/entailment pass is future work).

### Gap 3 — ranking is opaque by design

No mainstream engine lets a user see, for one specific request, why one
page ranked above another. Ranking transparency shipped as an explanation
in the press rather than as a runtime feature of the product.

**Covered in LYNX:**

- `explain=true` returns the full per-result signal vector (final score,
  lexical relevance, freshness, quality, authority, query context, spam
  score, applied penalties) in **both** JSON and HTML representations.
- The HTML SERP renders a per-result "Why this ranking?" disclosure with
  the same numbers, the ranking-config version they were computed under,
  and the explicit statement that every field comes from the document's
  stored signals — nothing about the searcher is used.
- It is opt-in; the default SERP stays compact. The numbers are escaped
  text, so the disclosure is not an injection surface.
- Operator-side deep explains (retrieval, shard, cluster, spam coverage)
  are token-gated via the separate `/api/v1/explain` surface.

### Suggested page blurb (single paragraph)

> LYNX v0.1.0 is out. It is the first release of a search engine with a
> fully owned crawl-index-rank-serve pipeline (no Bing/Google syndication
> anywhere in the loop), an explainable ranker that lets anyone open a
> "Why this ranking?" view per result with the exact signal vector, and an
> optional grounded-answer layer that only ever states what a verified
> citation supports — and refuses otherwise. Privacy is structural: no
> query logs, no cookies, no third-party assets, no profile to attack.

---

*Last updated: 2026-10-27, from the v0.1.0 release engagement (tagged
`v0.1.0` at https://github.com/anoneurx/lynx and published as a GitHub
release with the full rehearsal report).*
