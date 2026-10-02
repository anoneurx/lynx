# ADR 0004: Index Generation & Atomic Reload

**Date:** 2024-02-01
**Status:** Accepted
**Deciders:** Search Team, Platform Team
**Tags:** index, deployment, zero-downtime

## Context

Tantivy readers hold memory-mapped segment files.
To pick up a new index generation, readers must be reloaded.
Goal: **zero-downtime**, **sub-second** propagation, **no query failures**.

## Decision

**Generation-based immutable indexes + atomic symlink swap + Redis pub/sub reload signal.**

### Write Path (Indexer)

1. Build new generation in `gen-<N+1>/` (Tantivy `IndexWriter::commit()`)
2. `fsync` directory
3. Atomic symlink: `ln -sfn gen-<N+1> current`
4. `PUBLISH INDEX_RELOAD <N+1>` on Redis

### Read Path (API)

1. Subscribe to `INDEX_RELOAD` channel on startup
2. On message: `IndexReader::open(current_dir)` → new `Arc<IndexReader>`
3. Swap `Arc` in `DashMap<shard, Arc<Reader>>` (lock-free)
4. In-flight queries complete on old reader; new queries use new reader

### Guarantees

- **No mixed generations** per query (reader is immutable snapshot)
- **No downtime** (old reader stays alive until refcount = 0)
- **Sub-100ms propagation** (Redis pub/sub ~1ms + reader open ~50ms)
- **Rollback** = `ln -sfn gen-<N> current` + `PUBLISH INDEX_RELOAD <N>`

## Consequences

### Positive
- Simple, robust, proven (similar to Lucene `IndexWriter`/`SearcherManager`)
- Readers never block writers
- Easy debugging: `ls -la index/` shows all generations

### Negative
- **Storage overhead**: keep N generations (default 5) → 5× index size on shared volume
- **Reader memory**: each generation loads FST, norms, doc values into memory
- **Schema changes** require full rebuild (no live schema migration)

## Configuration

```toml
[index]
generations_to_keep = 5
reload_signal_channel = "INDEX_RELOAD"
reader_reload_timeout_ms = 5000
```

## Alternatives Rejected

| Approach | Reason |
| -------- | ------ |
| `SearcherManager` (Tantivy built-in) | Requires `IndexWriter` in same process; we separate indexer |
| Rolling restart API pods | Downtime, slow, loses connection affinity |
| Copy-on-write filesystem (ZFS/Btrfs) | Not portable, kernel dependency |

## Related

- ADR 0003: Tantivy Index
- ADR 0011: Sharding Strategy