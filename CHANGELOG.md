# Changelog

All notable changes to LYNX. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[semantic versioning](https://semver.org/spec/v2.0.0.html).

This project is in its **design phase**. No released version contains a running search
engine yet.

## [Unreleased]

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