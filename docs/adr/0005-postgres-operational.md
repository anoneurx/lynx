# ADR 0005: Postgres for Operational Data

**Date:** 2024-01-25
**Status:** Accepted
**Deciders:** Platform Team, DBA
**Tags:** database, postgres

## Context

Operational data needs:
- Crawl queue (millions of URLs, priority ordering)
- Page metadata (URL, title, content_hash, fetch_time, status)
- Indexer metadata (generation, doc count, segment info)
- User accounts, API keys, audit logs
- ACID transactions, complex queries, JSONB

Options: Postgres, MySQL, CockroachDB, FoundationDB, SQLite.

## Decision

**PostgreSQL 16** (Patroni for HA, PgBouncer for pooling).

## Consequences

### Positive
- **Mature, battle-tested** → 25+ years, massive ecosystem
- **Rich data types** (JSONB, arrays, ranges, tsvector)
- **Advisory locks** → leader election for indexer
- **Logical replication** → future CDC for analytics
- **Patroni** → auto-failover, synchronous replication
- **WAL-G** → continuous backup, PITR, S3 storage

### Negative
- **Connection overhead** → mitigated by PgBouncer (transaction pooling)
- **Write throughput** limited by single primary (mitigated: read replicas, partition crawl queue)
- **Vacuum** maintenance → automated with `autovacuum_vacuum_scale_factor=0.05`

## Schema Highlights

```sql
-- Crawl queue: priority queue with SKIP LOCKED
CREATE TABLE crawl_queue (
    url_hash      BYTEA PRIMARY KEY,
    url           TEXT NOT NULL,
    priority      INT NOT NULL DEFAULT 0,
    scheduled_at  TIMESTAMPTZ NOT NULL,
    attempts      INT NOT NULL DEFAULT 0,
    claimed_by    UUID,
    claimed_at    TIMESTAMPTZ
);
CREATE INDEX ON crawl_queue (priority DESC, scheduled_at) WHERE claimed_by IS NULL;

-- Pages: metadata + content hash for deduplication
CREATE TABLE pages (
    url_hash      BYTEA PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    title         TEXT,
    content_hash  BYTEA,
    fetched_at    TIMESTAMPTZ,
    status_code   INT,
    content_type  TEXT,
    lang          CHAR(3)
);
```

## Sizing

| Scale | Instance | Storage | Replicas |
| ----- | -------- | ------- | -------- |
| < 10M pages | db.r6g.xlarge | 500 GiB | 2 |
| 100M pages | db.r6g.2xlarge | 2 TiB | 2 |
| 1B pages | db.r6g.4xlarge | 6 TiB | 3 |

## Alternatives Rejected

| DB | Reason |
| -- | ------ |
| MySQL | Weaker JSONB, no advisory locks, less flexible indexing |
| CockroachDB | Higher latency, overkill for single-region |
| FoundationDB | Operational complexity, smaller talent pool |
| SQLite | No concurrency, no network |

## Related

- ADR 0006: Redis for Cache & Coordination
- ADR 0010: Crawler Architecture (queue in Postgres)