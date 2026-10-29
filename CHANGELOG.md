# Changelog

All notable changes to LYNX. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[semantic versioning](https://semver.org/spec/v2.0.0.html).

v0.1.0 is the release of the §Z implementation order end to end: a real crawl-index-
search loop with no third-party requests and optional, off-by-default Astra answers.
Later phases ([ROADMAP.md](./ROADMAP.md)) — scale, sharding, multi-region — are
post-v0.1 by design.

## [Unreleased]

## [0.1.0] — 2026-10-27

The first end-to-end version: crawl → index → search → SERP, with everything each
§Z step promised in commits shipped and gated (fmt, clippy, tests, link + privacy
checks), plus the optional-by-default Astra answer layer behind a kill switch,
token budget, circuit breaker, and fail-closed citation verification.

### Added — differentiation against the landscape (the gaps others leave open)

- **Per-request ranking transparency in the SERP** (`apps/api/src/serp.rs`,
  `docs/api/search.md` § Ranking transparency): no mainstream engine lets a user
  see why one result ranked above another — ranking opacity is structural
  (Google/Bing opaque; privacy alternatives mostly rent their organic links
  from Bing/Google syndication, e.g. DDG/Startpage/Ecosia/Qwant; only Brave and
  Mojeek really index). `explain=true` now surfaces the full per-result signal
  vector (lexical, freshness, quality, authority, context, spam, penalties)
  **in the HTML representation as a per-result "Why this ranking?" disclosure**
  with the ranking-config version and the statement that nothing about the
  searcher is used. JSON already carried the vector; the HTML view makes it
  human-legible, and it is opt-in — the default SERP stays compact.
- Installation-sized note in the README ("What makes LYNX different"): the
  three structural gaps — rented result lists, unverified AI citations
  (CJR Tow Center 2025: all eight AI search evaluations found citation
  problems; hallucinated citations rising), and ranking opacity — and what
  LYNX does for each, with honest v0.1 limits stated next to the claims.

### Added (implementation Step 22 — trust gate for the answer context)

- **`ai.min_source_trust`** (default `0.0` = off): a candidate whose remaining
  trust (`1.0 - spam_score`) is below the threshold is withheld from the head
  the answer plane may ground from, per-candidate — a clean SERP still answers,
  a fully-below-gate SERP degrades to the ordinary SERP-only response. A spammy
  page can still rank (ranking is relevance); it is just not handed to the
  model as something true (`docs/ai/astra.md` § Trust gate). Contracts with
  tests: the knob must stay within `[0, 1]`; an end-to-end suite films a
  spammy page being excluded from the grounding head at gate `0.7` and
  included at the default `0.0`.

### Added (implementation Step 21 — full-loop smoke over three real services)

- **`apps/api/tests/smoke.rs`** films the whole v0.1 loop the way a
  deployment runs it: the real `lynx-api` router over a **sealed Tantivy
  generation** in a temp dir, dialing the **real `lynx_ai::engine::AnswerEngine`**
  (with its real `OpenAiCompatibleModel`) behind `/v1/answer`, which talks to a
  canned model origin playing vLLM/Ollama (`POST /chat/completions` with the
  exact wire shape `OpenAiCompatibleModel` speaks — model, temperature 0,
  `response_format json_object`, usage accounting). Nothing mocks our own
  types; only the model origin is canned, which is what it is in production
  too. Properties verified end to end: a cited answer merges into the search
  response and its provenance points into the real index; the identical
  search is a `from_cache` hit served by the real `AnswerCache`; a model that
  ignores grounding fails closed into a SERP-only 200 (no `answer` key).

### Added (implementation Step 20 — real index bootstrap)

- **`lynx-api` serves from a sealed generation, not a hardcoded fixture**
  (`apps/api/src/reader.rs`): `index.dir` with a `generation.json` manifest is
  opened as the serving Tantivy index in every environment; a missing
  generation falls back to the development fixture in development/staging so
  `cargo run` works with no crawl; a production boot without one is refused
  with an operator-readable reason — serving an empty index to users would be
  the outage by another name. The boot log names which surface is being
  served. Config contract (with tests): production refuses a `/tmp` serving
  index (`index.dir`), because the generation is wiped on reboot; the
  shipped `config/production.toml` already pointed at `/var/lib/lynx/index`.
- Development fixture moved out of `main` into the boot module it supports,
  shared with the fixture's documented corpus. `lynx-indexer`'s `tantivy`
  feature is now carried by `lynx-api` (the reader needs the engine).

### Added (implementation Step 19 — the search plane dials the answer plane)

- **`GET /api/v1/search` merges verified answers** when the deployment opts in:
  config gained `ai.service_bind` (what the `lynx-ai` process binds — it no
  longer shares the public API's `api.bind` address), `ai.answer_service`
  (what the public API dials; empty keeps answers off) and
  `ai.answer_timeout_ms` (per-request deadline). The search handler grounds
  from the ranked head (`ai.max_sources` docs) and attaches a
  `SearchResponse.answer` block — verified text plus citation provenance —
  only when the answer plane produced one (`docs/api/search.md` § Grounded
  answer). **The SERP is never hostage to the answer plane**: transport
  failures, timeouts, malformed bodies, and typed refusals all degrade to the
  ordinary SERP within the deadline, and the HTML representation renders the
  answer as a cited block above the results (citation markers become links
  there; markers that resolve to no citation render as inert text, never
  injection).
- **Config contracts extended** (`lynx-config` validation, with tests): a
  malformed `ai.answer_service` (no http(s) scheme) never validates in any
  environment; production requires `https` for the dial address (it carries
  the active query text) and a parsable `ai.service_bind` when the answer
  plane is enabled.
- Fail-closed-by-construction in the client: `AnswerClient::answer` returns
  `Option` — `None` for every failure and refusal — and its unit suite +
  the end-to-end suite (`apps/api/tests/answer.rs`) film an answered call, a
  refusal, a dead endpoint, and a route that times out under the deadline.

### Added (implementation Step 18 — the Astra answer layer, optional by default)

- **`lynx-ai` crate end-to-end** (§Z Step 18; off by default and irrelevant to any
  deployment that leaves `ai.enabled = false`): a strictly-lowercase plane that
  summarises top-ranked sources and cites them, failing closed on every failure.
  Engine order: kill switch → daily token budget → circuit breaker → cache
  (hashed key over question + pinned model + grounded doc ids, TTL from config) →
  source preparation (unicode sanitise, injection screen, token-budgeted
  truncation with provenance offsets) → prompt assembly (compile-time system
  prompt + delimited `<data-source>` channel) → one model call → budget charge →
  citation verification → cache put. All timing through the injected
  `lynx_common::time::Clock`; budget day-roll and cache TTL are clock-driven.
- **Citation verification, fail-closed** (`services/ai/src/verify.rs`): every
  sentence must carry a valid `[n]` (`1..=max_ordinal`); trailing citations stay
  attached to the sentence they support (`. [2]` never leaks into the next
  sentence); uncited or bogus-index sentences are stripped; more-than-half
  stripped discards the whole summary with the constant fallback message
  (`"The retrieved sources could not be reliably summarised."`), and a wholly
  uncited output is that refusal rather than an empty-content technicality.
  `==50%` strip is the documented ceiling.
- **Injection defences** (`sanitise.rs`, `injection.rs`, `prompt.rs` + the
  adversarial suite `tests/injection.rs`): zero-width/BiDi/variation-selector
  removal and an intent-driven partial confusables fold *before* detection, so
  obfuscation evasions (`ign\u{200B}ore`, spaced `system :`, Cyrillic і/а
  lookalikes) are caught on the cleaned text; flagged sources are dropped
  entirely and ordinals renumber densely; a passage-closing-the-data-tag escape
  attempt is dropped; material that survives screening stays inside the labelled
  data channel (pinned by position in the assembled prompt); outputs with
  uncited injected instructions are refused by the verifier.
- **Providers** (`provider.rs`): a closed enum — `Mock` (deterministic, offline;
  extractive with `[n]` citations, scripted fixtures for the fail-closed tests)
  and `OpenAiCompatible` (one pinned endpoint, `temperature 0`, `top_p 0.1`,
  `response_format: json_object`, schema-validated `{"text": ...}` contract,
  char-estimate fallback when usage is omitted).
- **`lynx-ai` binary** (`src/bin/lynx-ai.rs`): `POST /v1/answer` taking the
  already-ranked SERP head from the search plane and returning either a verified
  answer or a typed refusal slug (`killed | budget_exhausted | circuit_open |
  no_sources | unreliable_summary | empty_output | provider_error`) — failures
  are refusals, never 5xx. Config is the layered TOML the other planes use
  (`config/example.toml` + `config/$LYNX_PROFILE.toml`, environment-validated).
- **Configuration contracts** (`lynx-config` validation, with tests): when
  `ai.enabled`, `provider = "none"`, `require_citations = false`, or
  `fail_closed = false` never validate — citation verification and fail-closed
  are non-negotiable in every environment (docs/ai/astra.md); production
  additionally requires a pinned model id and an `https://` endpoint.
- Docs: "v0.1 implementation scope (honest notes)" appendix in
  `docs/ai/astra.md` — content-blind sentence verification (bounded blast
  radius, not entailment), denylist detector with containment not depending on
  it, per-replica process-local controls, ~4-chars/token estimates.

### Added (implementation Step 17 — privacy hardening)

- **Global Privacy Control** (`packages/security/src/privacy.rs` + a
  `privacy_gpc` middleware in the API): `Sec-GPC: 1` requests get every
  `Set-Cookie` stripped from their response. LYNX sets no cookies by design
  (spec §N), so the strip is defence in depth against a future regression, and
  the integration tests pin the observable contract: a GPC client receives the
  byte-identical body, no cookie, and no extra headers — on both normal and
  GPC paths.
- **`no-store` tested everywhere** (§Z Step 17): error paths are now covered —
  404 fallback, 405 method-not-allowed, and the rate-limit 429 envelope
  (constructed *inside* the middleware stack) all assert
  `Cache-Control: no-store, no-cache, must-revalidate`, so no shared cache can
  ever hold a response body, including bodies that reveal a denial.
- **Production privacy check** (`lynx-config` validation): production refuses
  any `privacy.third_party_assets` value other than `forbidden` — the served
  CSP (`default-src 'none'`) allows none, so a loosened config can never
  actually work; test included. The always-on `query_logging` ban was already
  enforced; this closes the remaining §N surface.
- **Retention jobs live** (§Z Step 17): a declarative registry
  (`services/storage/src/retention.rs`) pairing every operational table with
  its `retention_policy` class, a fence test
  (`services/storage/tests/retention.rs`) failing the build if a migration
  creates a table without a class, a timed column that does not exist in its
  DDL, or a governance table carrying a "timed" column (governance is never
  clock-swept: `admin_audit_log` is append-only and `takedown_request` is the
  durable removal record); the executable sweep (`lynx-db sweep` — bounded
  parameterised deletes, reading `retain_days` live, skipping unmapped classes
  loudly via `lynx-db retention-check` rather than guessing); and
  `spawn_daily_sweeper` for the single-host run mode. Docs:
  `docs/privacy/retention.md` (mechanics and run modes) and
  `docs/privacy/data-inventory.md` (the itemised, privacy-facing inventory the
  minimisation claim rests on).
- **`check-links.sh` exemption removed**: the step-16 whitelist for
  `data-inventory.md`/`retention.md` is gone; the checker is now strict again
  (549 relative links, zero dead).

### Added (implementation Step 16 — observability & operational hardening)

- **Monitoring stack as code** (`infrastructure/monitoring/`): Prometheus scrape
  config (plus a `metric_relabel` drop rule as scrape-path defence in depth for the
  label ban list), alert rules for all six planes — availability (SLO + error-budget
  burn), search (latency p99 records, cache-hit ratio, rate-limit denial spike),
  crawler (frontier backlog, fetch stall, politeness denials, SSRF-block surge),
  index (generation staleness/36 h, build failure, generation size drop,
  takedown-queue age), queue (starvation correlation record), capacity (disk free
  %, memory, vault-access anomaly) — Alertmanager routing with plane grouping,
  severity-based paging, and inhibit rules so symptom alerts never double-page an
  already-firing plane; and a log pipeline that drops banned field names and
  strips request-line tails **before** persistence, with a hard 14-day hot retention.
- **Four Grafana dashboards** (overview, api, crawler, index) expressing only the
  permitted label set.
- **Canonical alert-code runbook index** (`docs/operations/runbooks.md`): every alert
  name (`LynxAPIHighErrorRate`, `LynxAPIErrorBudgetBurn`, `LynxCrawlerFetchStall`,
  `LynxIndexBuildFailed`, `LynxDiskFreeLowPct`, …) has a matching anchor here and
  routes into the relevant `docs/security/runbooks/*` procedure; alert annotations
  apply the same anchor string.
- **`docs/operations/observability.md`**: documented permitted metric labels with
  cardinality bounds, the CI-enforced ban list, the OTel knob contract
  (`tracing` / `tracing_endpoint` / `tracing_sample_ratio`), and the 14-day log
  retention rationale.
- **Privacy pages**: `docs/privacy/telemetry.md` (what telemetry exists, what cannot
  exist, config surfaces including the production 1% trace-sample cap and 14-day log
  ceiling), `docs/privacy/legal.md` (third-party content classes, takedown/
  erasure flow with the next-generation deletion guarantee).
- **`docs/security/headers.md`**: the canonical header set generated by
  `packages/security/src/headers.rs`, with the two documented cacheable exceptions
  (suggestions/status) and why everything else is `no-store`.
- **`docs/privacy/limitations.md`**: the honest list of ranking/coverage/product
  limits that follow from the no-user-data design.
- **Restore rehearsal** (`infrastructure/scripts/restore-rehearsal.sh`): Postgres
  `pg_dump` → scratch-restore → table-count comparison, and index-generation
  transfer with checksums plus the symlink swap against the compose stack; staging
  items not rehearsed locally are stated honestly. Evidence ledger added to
  `docs/operations/backup.md`.
- **Four CI checks** wired to the existing workflow: `scripts/check-privacy.sh`
  (banned metric/log field names in monitoring YAML and Rust emission sites, with
  `\b`-guarded patterns that exempt tantivy index-schema calls),
  `scripts/check-links.sh` (535 relative links across docs and root pages, zero
  dead), `scripts/check-config.sh` (every checked-in environment file parses over
  the example layer and validates for its declared environment via
  `packages/config/tests/config_files.rs`).
- **Configuration hygiene**: production caps (`log_retention_days ≤ 14`,
  `tracing_sample_ratio ≤ 0.01`, endpoint required when tracing is on) added to
  `lynx-config` validation with tests; un-parseable `[deploy.*]` tables moved out of
  `config/production.toml` into the deployment-shapes table in
  `docs/operations/scaling.md`; the production database URL is referenced as a
  secret instead of inline.
- **Link repair**: the ~103 pre-existing dead links in root docs, strategy docs and
  `doc/architecture/` (the ADR index stale double-numbering in
  `docs/architecture/specification.md` and `docs/architecture.md` is now redirected
  to the real `docs/adr/` records; `../` corrections in `docs/getting-started.md`,
  `docs/threat-model.md`, security runbooks; all six `docs/api/*` pages' phantom
  `openapi.yaml` now point at `API.md`; the RB links and `charts/ndcg_trend.png`
  pointers now resolve) — no dead-link exemptions were invented for this step;
  the only forward-facing gap is the step-17 `data-inventory.md` and
  `retention.md`, wired in `PRIVACY.md`/`CONTRIBUTING.md`/`DEVELOPMENT.md`
  intentionally and whitelisted in `check-links.sh` until they ship with step 17.

### Added (implementation Step 15 — quality harness)

- **`lynx-quality` crate** (§Z Step 15): graded-relevance metrics — NDCG@10
  (`2^rel−1` gains, position-discounted), MRR@10, Recall@100, plus top-10 hygiene
  ratios (duplicate-cluster share and spam-penalised share) — all total functions with
  unit-tested math.
- **Judged sets**: strict JSONL loader (one query per line, URL-keyed judgments,
  0–3 labels; malformed data is an error, never a silently skipped query) and the
  source-controlled **v0 judge set** (`fixtures/judge-set-v0.jsonl`, 9 queries) with its
  14-document fixture corpus (12 topical docs, one keyword-stuffed spam doc, one mirror
  duplicate).
- **Golden gates in CI**: `check_gates` pins the documented targets (NDCG@10 ≥ 0.65,
  MRR@10 ≥ 0.55, Recall@100 ≥ 0.80, duplicate < 0.20, spam < 0.10, p99 < 200 ms) and a
  test runs the v0 judged set through the production parse → retrieve → rank pipeline on
  every `cargo test`, so ranking regressions fail the build; `document` cross-check makes
  corpus/judge-set divergence a data-bug error. Current set: NDCG@10 0.966, MRR@10 1.0,
  Recall@100 0.889, duplicates 0, spam 0.0625, p99 < 200 ms — all gates pass.
- **Quality reporting**: `render_markdown` produces the documented "Search Quality
  Report" shape (headline metrics + zero-result coverage gaps).
- Docs: `docs/testing/quality-evaluation.md` and `docs/research/datasets.md` now point
  at the in-repo judge set and gates.

### Added (implementation Step 14 — suggestions service)

- **Suggestions completion pipeline** (§Z Step 14). `lynx-indexer` readers gained the
  `suggest_candidates(prefix, limit)` trait method with a parity-enforced contract: a
  term's doc frequency is per-field df summed across segments, max across the six text
  fields; title candidates are the first ≤ 4 lowercased tokens of a document title with
  the count of documents whose title opens with that phrase. Both backends implement it:
  the in-memory reader scans per-field token maps by prefix range, and the Tantivy reader
  scans each segment's term dictionary with a prefix byte automaton (`StartsPrefixAutomaton`,
  built on `tantivy-fst` — tantivy 0.22 ships no prefix automaton) plus a lazily-built,
  immutable title-phrase table stored in a `OnceLock`.
- **`lynx-suggestions` ranking** implements the documented score
  (`0.40·corpus-frequency + 0.25·title-bonus + 0.20·co-occurrence (0.0 in v0.1) +
  0.15·length-prior`), case-insensitive deduplication keeping the best-supported
  candidate, deterministic `(score desc, text asc)` ordering, and an output-side
  denylist as defence in depth (never on the index).
- **`GET /api/v1/suggestions`** serves typeahead per `docs/api/suggestions.md`: `q`
  (trimmed, ≤ 128 bytes), `limit` clamped 1–10 (default 8), `scope`
  (`all|titles|terms|related`, unknown scopes fall back to `all` with a
  `meta.notice`, and `related` answers empty with a notice — page-level in v0.1),
  interface language from `interface_lang` or `Accept-Language`, `Cache-Control:
  public, max-age=300` (the security middleware now treats it as a cacheable endpoint
  like `/status`), short prefixes (0–1 chars) answered with `200 []` without touching
  the index, and no write path anywhere in the handler.
- **Backend parity tests** for suggestions join the existing parity fence: identical
  candidate sets and working-set sizes between the in-memory and Tantivy readers, and
  unit/e2e tests for ranking, scopes, clamping, determinism, and the empty-answer
  contract.

### Hardened (crawler security)

- **IPv6 deny set completed.** Teredo `2001:0::/32` (obfuscated client part, denied whole
  as a protocol assignment) and IETF documentation `2001:db8::/32` are now classified and
  refused on every resolved address, closing the gap next to the existing NAT64/6to4
  unwrapping.
- **Dangerous ports cannot be allowlisted.** `FetchPolicy::new` rejects any allowlist
  containing a port below 1024 other than 80 and 443 (`PolicyError::DangerousPort`), so a
  config mistake cannot turn URL-driven fetches into probes of ssh/smtp/dns et al.
- **One budget for the whole fetch.** The downloader now runs a single absolute
  `Instant` deadline across every redirect hop; each hop's connect, header, idle, and
  request timeouts are clamped to what remains of the budget. Previously a chain kept
  a fresh full budget per hop — `max_redirects × total_timeout` of attacker-controlled
  wall-clock.
- **Multi-member gzip bodies decode fully.** `MultiGzDecoder` replaces the single-member
  `GzDecoder`; concatenated members (proxies, resumable downloads) no longer truncate
  after the first.
- **`Retry-After` and `Crawl-delay` are clamped** (`MAX_RETRY_AFTER_SECONDS` 3600,
  `MAX_CRAWL_DELAY_SECONDS` 86400): server-supplied delays are attacker-controlled and
  must not pin a URL or a host out for as long as the header claims.
- **`robots.txt` matcher input is bounded.** `GroupSelection::decision` truncates the
  request path to `MAX_PATH_LEN` on a UTF-8 character boundary before the
  `O(pattern × path)` glob, so an over-long URL cannot stretch matching work.
- **`dangerously_allow_private_addresses` warns.** The deliberate defeat of the SSRF
  gate now emits a `WARN` at construction, so a test-only builder leaking into
  production config is visible in logs.
- **New end-to-end tests**: non-web schemes (`file:`, `gopher:`, `javascript:`,
  `data:`), redirect-budget exhaustion against `/redirect/chain/:n`, the global budget
  cutting a patient idle timeout, and a 9-hop chain under one budget.

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

- The controls designed here are implemented, tested, and gated in this release: the
  layered SSRF defence (schemes, ports, blocked CIDRs in every encoding, per-hop
  redirect validation, DNS pinning — `services/crawler/tests/safety_gate.rs`), the
  privacy layer (rate limiting, GPC, `no-store`, query redaction), secret references
  only in configuration, and the prompt-injection boundaries in the Astra layer
  (`services/ai/tests/injection.rs`). Each maps to a numbered threat in the threat
  model.

### Known limitations

- The index is v0.1-scale small: the crawler, not a data centre, is the constraint.
  Coverage will be worse than commercial engines for years.
- Performance figures in the docs are targets unless labelled as measured; the measured
  figures in this changelog come from the in-repo fixture corpus, not the live index.
- Astra v0.1 citation verification is sentence-level and content-blind — it bounds
  blast radius, it does not prove truth (`docs/ai/astra.md` § v0.1 implementation
  scope).
- Link-graph authority, sharding, and multi-region are post-v0.1 by design; their exit
  criteria live in [ROADMAP.md](./ROADMAP.md).
- Open technical questions are listed in the specification rather than papered over.