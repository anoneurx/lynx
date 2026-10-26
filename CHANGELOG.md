# Changelog

All notable changes to LYNX. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[semantic versioning](https://semver.org/spec/v2.0.0.html).

This project is in its **design phase**. No released version contains a running search
engine yet.

## [Unreleased]

### Changed (performance)

- **`collection_stats` no longer rescans the corpus.** The Tantivy reader computes
  average field lengths once, straight from the segments' inverted-index token totals,
  caches it on the reader (`OnceLock`), and serves per-request reads from memory; the
  old path fetched and re-tokenised every stored document on every search.
- **Document frequency is a per-search lookup, not per candidate.** The old
  `term_frequencies_for` issued a full-corpus `Count` query for every query token on
  *every* candidate; DF is now computed once per query token and cached
  (`DocumentFrequencyCache`) for the whole candidate list.
- **Stored fields are tokenised once per candidate.** Field hits, per-term
  frequencies, and field lengths all derive from one `FieldTokenCounts` map instead of
  re-tokenising every field for every token (`O(tokens × fields × field length)`
  previously).
- **`GenerationBuilder::add` is O(1) amortised.** Documents update/remove through a
  position map instead of a linear `find`/`retain`, so a full re-index is linear
  rather than quadratic.

### Fixed (query/indexer correctness)

- **Backend parity enforced.** A new `services/indexer/tests/parity.rs` loads the same
  fixture corpus into the in-memory reference and the Tantivy reader and asserts the
  same result sets and `count()` over a matrix covering AND/OR, phrases (contiguity
  within one field only), punctuation/case folding, `intitle:`, `inanchor:`, `site:`
  (including subdomains), `inurl:`, `filetype:`, `ext:`, `lang:`, dates, freshness,
  negation, and empty queries. Any drift between the backends now fails tests, not
  users.
- **`must` + `OR` semantics aligned.** The in-memory reader required a should-term match
  even when must-terms were present; Tantivy made should optional there. Both now make
  should clauses additive when a mandatory clause exists, matching
  `docs/search/retrieval.md`.
- **Multi-token terms match in the in-memory reader.** A term like `c++` or
  `async-runtime` never matched before; terms are now split to their tokens in both
  backends, and candidates carry the same token-level term keys everywhere.
- **Phrases matched per field.** The in-memory reader matched phrases across a
  title/body boundary via a concatenated haystack; phrase matching is now a contiguous
  token run within a single field, like the Tantivy `PhraseQuery`.
- **Negation and coverage keys are token-level in the ranker.** `-GitHub` and `-c++`
  now actually penalise documents carrying the folded tokens; query-term coverage
  counts tokens, not raw terms.
- **`inanchor:` wired end to end** (parse → validate → both readers → operator name),
  anchor-text scored requirement mirroring `intitle:`.
- **`count(plan)` takes `now`.** Freshness and date windows filter the count with the
  same clock a search uses, so `meta.total` agrees with the returned page.
- **Tantivy filter pushdown and deterministic ordering.** Language and date filters are
  pushed into the Tantivy query (`TermQuery`/`RangeQuery` over a fast field), and the
  tantivy reader now re-sorts ties by doc-id ascending like the reference reader.
- **`site:` filter values tightened** (no ports, no leading/trailing hyphens); stray
  pasted URLs become literal terms instead of broken filters.
- **Index-time tokenisation equals query-time tokenisation.** The LYNX tokenizer is
  registered with Tantivy in every construction path (positions stored for phrases);
  full-width `Ｒｕｓｔ` now matches `rust` in the production reader too.

### Fixed (serving-plane hardening, serving audit 2026-10-26)

- **Security headers are now applied to every response** through a router-level layer
  (`packages/security/src/headers.rs` was defined but never wired); `/api/v1/status`
  keeps its documented `Cache-Control: public, max-age=60` and `Vary` is emitted.
- **Rate limiting is wired.** The rotating salted prefix-bucket limiter
  (`lynx-security::ratelimit`) now guards `/api/v1/search` at 30/min by default with
  the documented `RATE_LIMITED` envelope, `Retry-After`, and `X-RateLimit-*` headers;
  the bucket derives from the daily HMAC of the client network prefix, honouring
  `LYNX_API_TRUSTED_PROXY_DEPTH`, and nothing durable is stored.
- **`X-Request-Id` on every response.** A well-formed inbound id is echoed; anything
  else is replaced with a fresh UUIDv7 so it cannot inject into logs. Added the
  `NOT_FOUND` envelope fallback for unmatched paths.
- **Trace spans no longer carry the query string** — the custom span holds only the
  method and path, so raising the log level cannot leak query text into logs.
- **Search-endpoint extraction is tolerant.** The raw query string is parsed in-handler:
  malformed `count`/`offset` fall back to defaults and non-UTF-8 input is decoded
  lossily instead of failing out of the error envelope.
- **SERP hardening.** Only `http`/`https` URLs render as links (a `javascript:`
  entry in the index renders as inert text); pagination arithmetic is overflow-safe;
  truncation now sets `meta.notice` in JSON, and `explain` is honoured — signals are
  included only when `explain=1`, per `docs/api/search.md`.
- **`503`/`429` responses carry `Retry-After` headers** as `docs/api/errors.md` requires.
- Restored the end-to-end negation test that was dropped from `apps/api/tests`.

### Added

- **Architecture and design documentation.** System architecture with the two-plane
  separation (query plane vs. crawl plane), component responsibilities, search request
  flow, crawl flow, indexing flow, ranking pipeline, deployment topology.
- **Architecture specification.** The complete reference document at
  `docs/architecture/specification.md`: vision, goals, non-goals, components, data flow,
  data model, ERD, crawler, indexer, search, ranking, privacy, security, APIs, frontend,
  AI, observability, testing, deployment, disaster recovery, roadmap, open questions,
  architecture decisions, and the recommended v0.1 implementation order.
- **Threat model.** 35 threats across external attackers, malicious websites,
  infrastructure compromise, privacy, and availability, each with impact, likelihood,
  mitigation, and residual risk; plus an explicit out-of-scope section and a risk register.
- **Privacy model and commitments.** Five rules, four mechanisms, a data inventory with
  retention per category, the configuration switches that can weaken the model, a
  self-audit checklist, and an explicit limitations statement.
- **Data model.** ERD covering the crawl plane, content, index bookkeeping, governance,
  and quality evaluation, with retention classes and a schema-level guarantee that no
  table can hold a search query.
- **Query language grammar and operator set.** `site:`, `filetype:`, `intitle:`,
  `inurl:`, `"exact phrase"`, `-term`, `OR`, plus planned `inanchor:`, `ext:`,
  `before:`, `after:`, `lang:`. No user-supplied regex, by design.
- **Ranking design.** BM25F with field weights, freshness, quality, authority, spam
  penalties, and near-duplicate suppression; versioned weights; an explainability
  contract that returns a full signal breakdown with every result.
- **Crawler design.** Frontier, robots handling, downloader caps, parser bounds, politeness
  and budget policy, and the SSRF defence (layered, IP-authoritative, per-hop redirect
  validation, DNS pinning).
- **API contract.** REST endpoints, error taxonomy, rate limits, cursor pagination,
  API-key lifecycle, and an OpenAPI 3.1 source of truth.
- **Frontend design.** Route map, rendering strategy, privacy constraints, accessibility
  requirements, and a distinct visual identity.
- **Astra AI layer design.** Optional, isolated, retrieval-grounded answers with verified
  citations, and prompt-injection defences specified as an architecture property.
- **Observability plan.** Metrics catalogue with cardinality policy, structured logging
  with query redaction, tracing with sampling, dashboards, and alerts that require
  runbooks.
- **Testing strategy.** Unit, integration, golden, E2E, security, fuzz, and performance
  layers; hermetic fixtures; ranking metric gates; privacy invariants as tests.
- **Deployment stages.** Local, development, small, medium, and large, with reference
  sizing, scaling triggers, security posture, and pipeline.
- **Backup and disaster recovery policy.** What is reproducible, what must be backed up,
  RPO and RTO per stage, and rehearsal requirements.
- **Configuration system.** Layered TOML with strict validation, secret references only,
  and environment-specific rules that refuse unsafe combinations.
- **Repository scaffolding.** Cargo workspace, pnpm workspace, Makefile, local
  Docker Compose stack, `.env.example`, CI workflows (CI, quality gates, E2E, fuzz,
  performance, supply chain, images, staging, production, docs), and issue/PR templates.
- **Evaluation plan.** Precision@k, Recall@k, MRR, NDCG, freshness, duplicate rate, spam
  rate, index coverage, and a curated judged-query-set methodology that does not require
  logging user queries.

### Changed

- Architecture and threat-model documents moved into `docs/` so that all design
  documentation has one home. Links updated across the tree.

### Security

- No code to secure yet. The controls designed here (SSRF gate, privacy layer, secret
  handling, supply chain, admin isolation, prompt-injection boundaries) are specified and
  each maps to a numbered threat and a planned test.

### Known limitations

- No running service. No index. No crawler.
- Performance figures are targets, not measurements. They are labelled as targets
  everywhere they appear.
- Open technical questions are listed in the specification rather than papered over.