# `database/migrations` — SQL migrations

Forward-only, append-only, ordered by timestamp: `NNNN_name.sql`.

```text
database/migrations/
├── 0001_extensions.sql          pgcrypto, pg_trgm, btree_gin
├── 0002_privacy_foundation.sql  audit + retention tables
├── 0003_identities.sql          admin_user, admin_role, api_key
├── 0004_crawl_plane_core.sql    domain, host, url, crawl_frontier, budget
├── 0005_crawl_plane_ops.sql     crawl_attempt, ban, crawl_stat_daily
├── 0006_content.sql             document, document_version, link, robots_policy
├── 0007_index_bookkeeping.sql   index_manifest, shard, segment, dedup_cluster
├── 0008_governance.sql          abuse_report, takedown_request
├── 0009_search_quality.sql      quality_evaluation_run, relevance_judgement
└── 0010_ranking_config.sql      ranking_config_version
```

## Rules

1. **Immutable once merged.** Fix mistakes with a new migration. Editing a shipped
   migration fails CI (`schema_migrations.checksum` mismatch).
2. **Expand / contract.** Each migration is tagged `-- +migrate:expand` or
   `-- +migrate:contract`. Contracts are only merged after the preceding application
   version is fully deployed and drained.
3. **No long locks.** Any statement that would take a lock longer than 5 s on production
   data volumes must be split (add nullable → backfill in batches → set default → add
   constraint `NOT VALID` → validate → add constraint).
4. **Every migration ships with** the query plans it invalidates noted in the header, and
   a note on whether it needs a concurrent index build.
5. **No data migrations that are not idempotent** and resumable. A migration that fails
   half-way must be safe to re-run.
6. **Foreign keys are added deliberately.** Some hot-path references use a logical
   reference plus an integrity job instead of a hard FK, to keep write contention
   predictable. The choice is documented per table in `database/schemas/`.
7. **No personal data** may be introduced by a migration without a matching row in
   [docs/privacy/data-inventory.md](../../docs/privacy/data-inventory.md). CI greps new
   migrations for likely-PII column names and fails for review.

## Local workflow

```bash
make migrate          # apply all pending migrations
make migrate-status   # show applied/pending
make migrate-new NAME # scaffold a timestamped file with the required header
make db-reset         # drop, recreate, migrate, seed (local only)
```