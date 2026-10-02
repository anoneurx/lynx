# Roadmap — LYNX

A roadmap with exit criteria, not aspirations. Each phase lists what it delivers and what
must be true to move on.

## Current state

**Phase 0 in progress.** Architecture, threat model, data model, ADRs, configuration,
repository structure, CI, and evaluation plan are written. No production code yet.
See [docs/architecture/specification.md](./docs/architecture/specification.md) for the
full design and §Z for the implementation order.

## Phase 0 — Architecture *(current)*

| Deliverable | Status |
| ----------- | ------ |
| Repository structure with per-directory documentation | done |
| System architecture, component boundaries, two-plane split | done |
| Data model (ERD, retention classes, no-user-data guarantee) | done |
| Threat model with residual risks | done |
| Privacy model and data inventory | done |
| Technology decisions (ADRs 0001–0021) | proposed |
| Query language grammar | done |
| Ranking design with weights | done |
| API contract and error taxonomy | done |
| Observability and metrics catalogue | done |
| Testing strategy, quality metrics, benchmark plan | done |
| Deployment stages, DR, backup policy | done |
| Local dev stack, Makefile, configs, CI pipelines | done |
| Documentation map and ADRs skeleton | done |

**Exit criteria:** ADRs accepted; every open question in the specification has an owner and
a first experiment scheduled; a reviewer outside the core team can follow the design
without asking what anything means.

## Phase 1 — Search prototype

Goal: a working, measured search engine before there is a crawler. The ranking target is
measured against a seeded corpus, so crawler work is measured rather than assumed.

- Query processor: lexer, parser, AST, operators, normalisation, spell correction, expansion
- `SearchIndexReader` trait, an in-memory fake, and a real Tantivy reader behind a flag
- Indexer: normalise → tokenise → write → commit
- BM25F ranker with field weights, phrase queries, filters, freshness, quality
- `/api/v1/search` end to end with the full middleware stack and standard errors
- Web SERP: `/`, `/search`, operators help, theme, accessibility, pagination
- Evaluation harness: judged query set, NDCG@10 / Recall@50 / MRR@10, golden rankings
- Seeded fixture corpus so every ranking change is a reviewable diff

**Exit criteria**
- p95 < 400 ms on the seeded corpus
- Every operator parses, validates, and executes; malformed input yields typed errors
- Every served result carries a full signal breakdown
- NDCG@10 on the judged set exceeds the v0.1 baseline and the metric gate runs in CI
- The privacy suite passes: no query substring in any log, metric, trace, or DB row

## Phase 2 — Crawler

Goal: crawl responsibly, and never touch internal infrastructure.

- Frontier: queue, priority, dedup, scheduling, budgets, bans, `SKIP LOCKED` claim
- robots.txt: fetch, parse, cache, crawl-delay, user-agent groups, failure semantics
- Fetch safety gate: schemes, ports, DNS + blocked CIDRs (every encoding), per-hop redirect
  validation, DNS pinning. **Security review; fuzz coverage.**
- Downloader: streaming fetch, byte/decompression/deadline caps, backoff, `Retry-After`,
  ban escalation, no cookies, no JS
- Parser: bounded HTML → text/links/metadata; sanitisation; signatures; spam features
- Indexer pipeline: content-addressed documents, document versions, dedup clusters
- Politeness: per-host concurrency, page and byte budgets, trap detection
- Fixture origin server that behaves adversarially, for tests and local development
- Operational surfaces: queue depth, crawl metrics, error taxonomy

**Exit criteria**
- The SSRF matrix (all blocked ranges, all encodings, every redirect position) passes, and
  no test touches the public internet
- Robots compliance is 100 % on the fixture corpus and on a small live sample
- No fetch exceeds its byte, ratio, or deadline cap under the adversarial fixture server
- Parse succeeds on the malformed-HTML corpus without panic, hang, or unbounded allocation
- A full crawl of the seed set produces an index that returns sensible results for the
  judged query set

## Phase 3 — Real search engine

Goal: quality that a user would choose twice.

- Scale the crawl: more workers, better frontier prioritisation, sitemaps and feeds
- Link graph and PageRank authority, domain-capped, with topic-neutral teleport
- Near-duplicate clusters: SimHash + MinHash, demotion rather than deletion
- Full operator set: `site:` `filetype:` `intitle:` `inurl:` `"phrase"` `-term` `OR`
  `inanchor:` `ext:` `before:` `after:` `lang:`
- Quality and spam signals: stuffing, doorway clusters, link-farm features, cloaking variance
- Freshness tuning: publication-date extraction, per-topic decay half-lives
- Suggestions from corpus statistics (never from query logs)
- Search quality reports on a schedule, with a documented methodology
- Multi-language: language detection, per-language stopwords and stemmers, per-language index fields

**Exit criteria**
- Measured NDCG@10 within reach of a stated baseline for the judged set
- Duplicate rate in the top 10 below the target; spam-flagged rate below the target
- Median age of top-10 results for time-sensitive queries below the freshness target
- Index coverage of the sampled popular-domain set at or above the documented target
- A published quality report showing methodology, numbers, and known limitations

## Phase 4 — Privacy hardening

Goal: the privacy model survives contact with production.

- Query-minimisation audit of every path that touches a request
- Production privacy verification: logs, metrics, traces, database, edge configuration
- GPC and other signal handling end to end
- Data inventory completeness check in CI, per column, with retention enforced by jobs
- Retention and purge automation for every class, with alerts on failure
- Rate-limit salt rotation exercised in production and verified in metrics
- Privacy model published on the site, versioned with the code
- Third-party review of the model (invited, not assumed)

**Exit criteria**
- Every stored field has an owner, purpose, retention, and encryption method; nothing is
  stored without a row in the inventory
- Automated purge jobs verified against seeded expired records
- External review of the privacy model completed and findings published, including the
  ones that are uncomfortable
- No query text anywhere in the system, demonstrated by an automated audit rather than
  assertion

## Phase 5 — AI (Astra)

Goal: grounded answers that never outrun their sources.

- Retrieval-grounded answer generation over the top-ranked results
- Passage extraction with provenance offsets; bounded context; per-source truncation
- Citation generation and verification; numeric consistency checks
- Prompt-injection defences as architecture, not prompt wording; adversarial test corpus
- Trust scoring so low-trust sources can be excluded from the AI context
- Cost controls: token budget, cache, per-bucket limits, circuit breaker, kill switch
- Evaluation against a query set measuring citation correctness, not just fluency

**Exit criteria**
- Zero uncited claims in the evaluation corpus; unsupported claims are suppressed rather
  than emitted
- The prompt-injection corpus produces no instruction-following failures from page content
- Search results remain correct and fast with Astra enabled and disabled
- Answer latency and token cost within budget; kill switch verified

## Phase 6 — Scale

Goal: grow without a rewrite.

- Distributed crawl: more workers, preemptible capacity, resumable frontier state
- Index sharding with per-shard top-K and global rerank; shard coordinator
- Reader replication and failover
- Postgres partitioning, read replicas, archival of cold crawl history
- Region awareness for latency and for disaster recovery
- Cost controls, budget alerts, capacity planning from recorded measurements
- Multi-region DR, exercised not just documented

**Exit criteria**
- Stated p95 latency maintained at 10× the corpus of the previous stage
- A region loss meets the stated RTO in a rehearsed drill
- Every scaling trigger in DEPLOYMENT.md has a recorded measurement behind it

---

## What is explicitly not planned

From [docs/architecture/specification.md §3](./docs/architecture/specification.md#3-non-goals):
advertising, user accounts, personalisation, search history, full-page HTML archiving,
JavaScript execution in the crawler, PDF extraction in v0.1, distributed infrastructure
before single-node validation, and any claim of absolute anonymity. Each of these would
need its own ADR and a stated reason to change its mind.

## How priorities are decided

A proposal enters the roadmap when it has: a measured problem, a hypothesis, an experiment
or benchmark that can falsify it, and a statement of what it costs in privacy, complexity,
or honesty. "It would be nice" is a fine reason to try something in `research/`; it is not
a reason to ship it.