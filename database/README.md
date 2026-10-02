# `database/` — persistent state

**PostgreSQL 16** is the operational store. The search index (Tantivy) is a separate,
derived artifact — see [ADR-0002](../docs/adr/0002-search-index.md) and
[ADR-0004](../docs/adr/0004-database.md).

| Path | Contents |
| ---- | -------- |
| [`migrations/`](./migrations/) | Forward-only SQL migrations, applied in order, never edited after merge |
| [`schemas/`](./schemas/) | Schema documentation, ERD source, and rationale — the human-readable contract |
| [`seeds/`](./seeds/) | Development seed data: seed domains, blocklists, test fixtures. **No production content** |

## What lives in Postgres

- Crawl plane operational state: domains, hosts, URLs, frontier queue, crawl attempts,
  documents, document versions, link graph, robots policies, budgets, bans.
- Index-generation bookkeeping: shard and segment manifests, commit watermarks, dedup
  cluster assignments, term statistics needed for authority/idf beyond what the index holds.
- Identity and governance: `api_key`, `abuse_report`, `takedown_request`, `admin_user`,
  `admin_audit_log`, `admin_role`.

## What never lives in Postgres

- Search queries, query-derived tables, session records, user profiles, cookies, or any
  per-user data. There is no table in this schema where a user's search could be recorded.
- The full text corpus at scale *and* the inverted index — those are Tantivy's job.
  Postgres holds extracted text per document version for re-indexing and takedown, not
  an inverted index.

## Migration policy

1. Migrations are forward-only and append-only. A shipped migration is immutable; a
   mistake is fixed by a new migration.
2. Every migration declares whether it is **expand** (schema change only, backward
   compatible with the previous application version) or **contract** (drops or renames
   something, released only after the application version that stopped using it is fully
   rolled out). This keeps rolling deploys safe.
3. No migration may lock a large table for longer than 5 s. Adding a non-null column to a
   big table requires the expand/contract dance with a default and a backfill job.
4. Every migration has a tested `down` path for local development only, even though
   production roll-forward is the real procedure.
5. `schema_migrations` records the checksum; drift is a startup failure, not a warning.
6. Seed data is idempotent and never references third-party content.

## Operational notes

- Query patterns are covered by named prepared statements (`sqlc`-generated or equivalent)
  with `EXPLAIN (ANALYZE, BUFFERS)` baselines in `docs/operations/capacity.md`.
- The heavy tables (`crawl_attempt`, `link`, `document_version`) are range-partitioned by
  time or by hash to keep index maintenance predictable.
- Search-path-relevant tables use covering indexes tuned against real plans, not guesses.
- The database role used by the crawler is separate from the API read role, which is
  separate from the admin role. Least privilege is enforced by grants, not convention.