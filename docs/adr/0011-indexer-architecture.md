# ADR 0011: Indexer Architecture

**Date:** 2024-03-20
**Status:** Accepted
**Deciders:** Search Team, Platform Team
**Tags:** indexer, architecture, batch

## Context

Indexer requirements:
- Build Tantivy index from Postgres pages + S3 content
- Incremental (new/updated pages) + full rebuild
- Single writer (Tantivy `IndexWriter` not thread-safe for commit)
- Generation-based atomic publish (ADR 0004)
- Handle 1B+ documents
- Run periodically (daily) or on-demand

## Decision

**Single-leader indexer** (Patroni-style advisory lock in Postgres) with:
- Multi-threaded `IndexWriter` (one thread per shard/partition)
- Streaming from Postgres + S3 (no local buffer)
- Configurable merge policy (logarithmic for speed)
- Progress tracking in Postgres (`index_builds` table)

### Leader Election

```sql
-- Acquire lock (blocking)
SELECT pg_advisory_xact_lock(1234567890);  -- constant per cluster

-- Release on commit/rollback (auto)
```

### Build Process

```rust
async fn build_generation(&self) -> Result<u64> {
    let gen = self.next_generation().await?;  // MAX(generation) + 1
    let mut writer = IndexWriter::new(gen_dir, self.threads)?;

    // Stream pages in URL-hash order (better segment locality)
    let mut stream = sqlx::query_as!(Page, "
        SELECT * FROM pages
        WHERE indexed_generation < $1
        ORDER BY url_hash
    ", gen).fetch(&self.pg);

    while let Some(page) = stream.try_next().await? {
        let content = self.s3.get(&page.content_hash).await?;
        let doc = self.page_to_tantivy_doc(&page, &content)?;
        writer.add_document(doc)?;
        if writer.num_docs() % 10000 == 0 {
            writer.commit()?;  // Periodic commit for crash recovery
            self.update_progress(gen, writer.num_docs()).await?;
        }
    }

    writer.commit()?;  // Final commit
    writer.wait_merging_threads()?;

    // Atomic publish
    self.publish_generation(gen).await?;
    Ok(gen)
}
```

### Merge Policy

```toml
[indexer]
merge_policy = "log"  # logarithmic: fewer, larger segments
# merge_policy = "no_merge"  # for speed, manual merge later
```

## Consequences

### Positive
- **No writer conflicts** (single leader)
- **Crash recovery**: Periodic commits + progress table → resume from last commit
- **Scalable**: Threads = CPU cores; S3 bandwidth usually bottleneck
- **Observable**: Progress in Postgres, metrics (docs/s, merge time)

### Negative
- **Single writer** → vertical scaling only (mitigated: sharding ADR 0011)
- **Build time** ~linear with corpus (20 hrs for 1B docs on 8 vCPU)

## Sharding (Future)

When > 1B docs: split by `url_hash % N`, N independent Tantivy indexes.
Router in API: `shard = hash(url) % N`.

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| Distributed Tantivy (Quickwit style) | Overkill for current scale; adds complexity |
| Multiple indexers writing same index | Tantivy `IndexWriter` not safe for concurrent commit |
| Incremental only (no full rebuild) | Schema changes require full rebuild; segment fragmentation |

## Related

- ADR 0003: Tantivy
- ADR 0004: Index Generation & Reload
- ADR 0011: Sharding Strategy