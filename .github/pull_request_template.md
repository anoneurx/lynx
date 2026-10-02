<!--
  Pull request template for LYNX.
  Keep it short enough that people actually fill it in. Every section maps to a
  review gate in CONTRIBUTING.md; the gates that can block a merge are marked.
-->

## What and why

<!-- What does this change, and what problem does it solve? Link the issue. -->

Fixes #

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Ranking change (see the ranking section below — **required** for any weight or signal change)
- [ ] Crawler change (see the safety section below — **required**)
- [ ] Schema / migration change
- [ ] Documentation only
- [ ] Infrastructure / build
- [ ] Dependency update

## Checklist

### Required for every PR

- [ ] `make verify` passes (fmt, clippy, lint, typecheck, tests, security, ranking gates)
- [ ] Tests added or updated, including the **negative** case where behaviour changed
- [ ] Documentation updated where behaviour or configuration changed
- [ ] No new secrets, credentials, or third-party endpoints
- [ ] No new dependency without a note below
- [ ] `scripts/check-privacy.sh` passes — no query text in logs, metrics, traces, or DB

### Privacy review (blocking for anything touching the query path)

- [ ] No new column holds data that must appear in `docs/privacy/data-inventory.md`
- [ ] No new field derives from query text in a log, metric label, trace attribute, or error message
- [ ] Retention period defined for anything newly stored
- [ ] No third-party request, font, script, or analytics added to the web app
- [ ] Changes to `config/production.toml` under `[privacy]` include a new ADR

### Security review (blocking for `packages/security`, `services/crawler/safety.rs`, auth, and network code)

- [ ] Threat model cross-referenced (which threat IDs apply?)
- [ ] Negative tests included — the case that must fail
- [ ] Input validation and bounds explicit (no unbounded allocation, loop, or recursion)
- [ ] No `unwrap`/`expect`/`panic` in new code paths
- [ ] Property or fuzz coverage if this parses untrusted input

### Ranking changes (blocking for ranking changes)

- [ ] `ranking.config_version` bumped in `config/production.toml`
- [ ] Golden diff reviewed — which documents moved and why
- [ ] Quality metric report attached: NDCG@10, Recall@50, MRR@10, duplicate rate, spam rate
- [ ] p95 search latency change stated
- [ ] Rationale for each weight change documented in `docs/ranking/`

### Schema changes (blocking for migrations)

- [ ] Migration is expand-only in this PR (contract comes in a later release)
- [ ] No lock longer than 5 s on production-sized tables
- [ ] `database/schemas/` and the ERD updated, or an explanation of why they are unchanged
- [ ] Retention class assigned to every new table

### Performance (state the impact)

- [ ] Change is expected to be neutral, and why
- [ ] Or: measured impact on search p95 / indexer throughput / crawler throughput
- [ ] No regression > 20 % in any budget without a documented waiver

### Dependencies (leave empty if none)

| Dependency | Why | Licence | Advisory status |
|------------|-----|---------|-----------------|
| | | | |

## Breaking changes

- [ ] None
- [ ] API response schema changed — documented in `CHANGELOG.md` and `API.md`
- [ ] Config key changed — migration path documented in `DEVELOPMENT.md`
- [ ] Index format changed — full re-index required, noted in the release checklist

## Screenshots (frontend only)

## Checklist for the reviewer

- [ ] I checked that no query text can reach a log, metric, trace, or database row
- [ ] I checked that untrusted input is bounded on every path
- [ ] I checked that the tests would fail if this change were reverted