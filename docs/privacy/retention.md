# Retention

How long each thing exists, and what makes each expiry real — a policy table
nobody can find, a sweeper nobody runs, and a promise nobody can verify are
different artifacts, and **all three are required**. The inventory is in
[data-inventory.md](./data-inventory.md); this page is the mechanics.

## The three artifacts

1. **The declared policy** — migration `0002_privacy_foundation.sql` creates
   `retention_class` (an enum) and `retention_policy` (one row per class,
   `retain_days`, `NULL` meaning indefinite). Operators tune `retain_days`
   per class; classes and their semantics:
   - `crawl_permanent` — life of the entry in the corpus.
   - `crawl_temporary` — 30–90 days; diagnostic crawl history.
   - `governance` — legal / anti-abuse records; **never clock-swept**.
   - `aggregate` — metrics rollups.
   - `no_user_data` — contains no personal data by construction.
2. **The registered fate** —
   [`services/storage/src/retention.rs`](../../services/storage/src/retention.rs)
   lists every operational table with its class and, where applicable, its
   timed column and the justification in words. A test
   (`tests/retention.rs`) fails the build if a table exists in
   `database/migrations/` without a row here, or if a registered timed column
   is not actually in the table's DDL — so retention decisions happen when the
   table is created, not after data already exists.
3. **The job** — `lynx-db sweep` executes the policy (see below).

## The sweeper

`lynx_db sweep` (the `services/storage` binary):

- Reads `retention_policy` live. A missing class, or `retain_days = NULL`, means
  that class's tables are **skipped, not guessed**: the query contract here is
  *"never delete by default."*
- First runs `retention-check`; with policy violations it **refuses to sweep**,
  because a half-wired retention story is worse than a loudly absent one.
- Deletes with bounded, parameterised statements:
  `DELETE FROM <table> WHERE <col> < now() - make_interval(days => $1)` —
  idempotent, restartable, no side effects.

Currently two tables are clock-swept. Their bounds come from
`database/seeds/seed.sql` (development) and are tuned per environment by
operators writing to `retention_policy`:

| Table | Class | Column | Seeded default |
| ----- | ----- | ------ | -------------- |
| `crawl_attempt` | `crawl_temporary` | `started_at` | 90 days |
| `crawl_stat_daily` | `aggregate` | `day` | 396 days (13 months) |

Everything else is permanent, deleted as a *domain action*, or
never-swept-governance. In particular:

- `admin_audit_log` — append-only by trigger design. There is deliberately no
  update or delete trigger; the record of an administrative action must not be
  editable or erasable by the party that took it.
- `takedown_request` — the durable record that content was removed. Deleting
  this table's row would let the crawler replay the URL; the *content* removal
  is the deletion, the record stays.
- `abuse_report` — the field-encrypted contact details are erased on verified
  request (see [legal.md](./legal.md)), never on a clock.
- The dev seed lists `governance` with a 730-day value, but the sweeper contract
  excludes the `governance` class from clock deletion entirely: that number
  only takes effect if a future ADR explicitly opts the class in.

## Running it

```text
lynx-db retention-check     # verify every timed class has a bound; exit 1 otherwise
lynx-db sweep               # one pass; prints per-table rows deleted
```

| Deployment | How the job runs |
| ---------- | ---------------- |
| Kubernetes (production) | a `CronJob` running `lynx-db sweep` daily, with `DATABASE_URL` from the same secret reference as the services; failures page via the `LynxIndex*` family only if the queue backs up — the sweep itself is quiet |
| Single host (compose/dev today) | `lynx_storage::retention::spawn_daily_sweeper(pool)` from a long-running process, or a cron/systemd timer on `lynx-db sweep` |
| Manual / audit | `lynx-db sweep` before a backup so an archive does not preserve rows already past retention |

## Backup interaction

A retention sweep runs **before** the nightly backup, so an encrypted archive
never holds rows already past their class bound; the 35-day backup ceiling then
acts as the outermost retention bound. The restore rehearsal
([backup.md](../operations/backup.md)) reconstructs tables and row counts — a
rehearsed restore is also the check that the sweeper did not over-delete:
the count comparison makes any unexplained shrinkage visible.

## What this does *not* do

Retention is about the **corpus** and **operational** state. It is not the
erasure path — a takedown or an erasure request acts on the current and future
index generations and on the named tables, with verification
([legal.md](./legal.md)). And it never touches telemetry: logs are dropped at
14 days by the pipeline itself, not by this job.
