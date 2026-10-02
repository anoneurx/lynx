# PostgreSQL — operational state, and nothing else

Postgres holds what Tantivy cannot: mutable state, relationships, and queues. It holds **no
search index and no user data**, and keeping that boundary sharp is what makes the privacy
architecture auditable.

```text
┌─────────────────────────────────────────────────┐
│ Postgres (operational)                          │
│   frontier · links · tombstones · robots ·      │
│   dedup signatures · quality metrics            │
├─────────────────────────────────────────────────┤
│ Tantivy (the product)                           │
│   documents · terms · positions · ranking fast  │
│   fields · index statistics                     │
└─────────────────────────────────────────────────┘
no query tables · no user tables · no analytics tables
```

| Document | Contents |
| -------- | -------- |
| [schema.md](./schema.md) | Every table, column, constraint, and why |
| [erd.md](./erd.md) | Relationship diagrams |
| [migrations.md](./migrations.md) | Migration strategy, naming, reversibility |

## Why Postgres and not something else

| Requirement | Why Postgres fits |
| ----------- | ----------------- |
| Concurrent queue consumers | `FOR UPDATE SKIP LOCKED` is exactly the frontier's access pattern |
| Relational integrity | Links, hosts, and documents relate; a graph store would not simplify it |
| JSONB where the shape varies | Content metadata, extract features, quality labels |
| Mature tooling | Backup, replication, and monitoring are solved problems |
| Full-text for diagnostics | Enough to debug the crawler without adding another engine |

The choice that was **not** made: Postgres full-text search as the query engine. Tantivy is the
index ([../indexing/](../indexing/)), and having two full-text implementations would mean two
sets of ranking behaviour and one place where they disagree.

## The queue pattern

The frontier is the hot table, so its access pattern gets its own treatment.

```sql
-- claim a batch of work
WITH claimed AS (
  SELECT id
  FROM frontier
  WHERE state = 'pending'
    AND available_at <= now()
    AND (host_id IS NULL OR EXISTS (
      SELECT 1 FROM crawl_host h
      WHERE h.id = frontier.host_id
        AND h.next_request_at <= now()
        AND h.requests_in_window < h.rate_limit
    ))
  ORDER BY priority DESC, available_at ASC, id ASC
  LIMIT $1
  FOR UPDATE SKIP LOCKED
)
UPDATE frontier f
SET state = 'claimed', claimed_by = $2, claimed_at = now()
FROM claimed c WHERE f.id = c.id
RETURNING f.*;
```

Three properties that matter:

- **`SKIP LOCKED`** lets N workers claim disjoint batches with no coordination and no blocking.
  A worker crash leaves rows in `claimed`; a reaper returns them after a timeout.
- **The host join is the politeness enforcement**, in the query itself rather than in
  application code. A worker cannot exceed a host's rate, because the claim will not return
  rows it is not entitled to.
- **`ORDER BY priority DESC, available_at ASC`** implements backoff naturally: a repeatedly
  failing URL is re-scheduled into the future instead of being retried immediately.

## Connection pooling

```toml
[database]
max_connections       = 100
statement_timeout     = 15_000     # ms
lock_timeout          = 3_000
idle_in_transaction_session_timeout = 30_000
```

| Component | Pool size | Rationale |
| --------- | --------- | --------- |
| API | 2–4 | Read-mostly, and it barely uses Postgres |
| Crawler | 16–32 | One per worker, plus frontier writes |
| Indexer | 8 | Bulk link writes |
| Tools | 2 | Migrations and diagnostics |

`lock_timeout` is set below `statement_timeout` on purpose: a lock wait should fail fast with a
retryable error rather than consume the whole statement budget.

## Read/write separation

| Consumer | Access |
| -------- | ------ |
| API | Read-only role |
| Search | Read-only role |
| Crawler | Read-write on frontier, robots, hosts, tombstones |
| Indexer | Read-write on links, signatures, quality labels |
| Migrations | Schema owner, used only at deploy time |

The search role having no write access anywhere is the point:
[../security/README.md](../security/README.md) notes that `lynx-search` cannot write a query
anywhere, and this is where that is enforced rather than merely intended.

## What is deliberately absent

| Not in Postgres | Where it is | Why |
| -------------- | ------------ | --- |
| Document text, terms, positions | Tantivy | Postgres full-text cannot do the field fusion or the performance |
| Query logs | **Nowhere** | [../privacy/privacy-model.md](../privacy/privacy-model.md) |
| User tables | **Nowhere** | No accounts |
| Analytics events | **Nowhere, off by default** | Opt-in ranked results only, when enabled |
| Raw HTML | **Nowhere** | Discarded after parsing |
| PageRank vectors | Tantivy fast fields | Read at query time for every candidate |

The PageRank decision is the interesting one. It could live in Postgres as a float column, but
then every ranked candidate needs a database lookup, and a 2000-candidate ranking would do 2000
round trips. Storing it as a fast field means the whole signal set reads from one index segment.

## Connection to the index

```text
crawl  ──▶ parsed doc ──▶ indexer ──▶ Tantivy
              │                            │
              └──▶ links, signatures        └──▶ doc_id is the join key
                   quality labels                  in Postgres too
```

`doc_id` is the join key in both systems, assigned once by the indexer and never reassigned. A
ULID, so ids are roughly time-ordered, which makes range scans on `first_seen` behave.

## Retention

| Table | Retention | Mechanism |
| ----- | --------- | --------- |
| `frontier` | Until dequeued | Deleted on success; kept as history on permanent failure |
| `crawl_host` | Indefinite | Needed for politeness accounting |
| `robots_cache` | Per spec | TTL expiry |
| `link` | Index lifetime | Source of the PageRank graph |
| `content_signature` | 90 d | Rolling window for cluster detection |
| `tombstone` | Indefinite | Deletion must be provable |
| `quality_label` | Indefinite | Trend tracking |
| `dedup_signature` | Indefinite | Cluster identity |
| `crawl_log` | 30 d | Diagnostic only |
| `analytics_result` | 30 d | Opt-in only; off by default |

Tombstones and signatures are retained indefinitely on purpose: a deletion that expires is a
deletion that can be undone by a replay.

## Backup

| Property | Setting |
| -------- | ------- |
| Method | Continuous WAL archiving plus daily base backups |
| RPO | ≤ 5 minutes |
| RTO | ≤ 2 hours |
| Retention | 30 d daily, 90 d weekly |
| Encryption | At rest, keys in KMS |
| Verification | Restored weekly into an isolated environment and checked |

Weekly restore verification is the control that matters. An unverified backup is a hypothesis.

## Conventions

| Convention | Rule |
| ---------- | ---- |
| Identifiers | UUID v7 for entities, ULID for documents |
| Timestamps | `timestamptz`, always UTC, never `timestamp` |
| Enums | PostgreSQL enum types where the set is closed and stable |
| JSON | `jsonb` for variable-shape data only, with a documented shape |
| Money | Not applicable |
| Text | `text` with a documented collation; no `varchar(n)` guessing |
| Soft deletes | Only where the history matters; otherwise `DELETE` |
| Nullability | `NOT NULL` by default; nullable only with a stated reason |

Nullable columns are the usual source of undocumented semantics, so the default is not nullable
and a nullable column carries a comment explaining what null means.

## Related

- [../indexing/](../indexing/) — where the index itself lives
- [../operations/backup-and-recovery.md](../operations/backup-and-recovery.md) — procedures
- [../crawler/frontier.md](../crawler/frontier.md) — the queue's semantics