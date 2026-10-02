# ADR 0003: Tantivy as Search Index Library

**Date:** 2024-01-20
**Status:** Accepted
**Deciders:** Tech Lead, Search Team
**Tags:** search, index, library

## Context

Need an embedded, high-performance inverted index library.
Options: Tantivy (Rust), Lucene (Java via JNI), Meilisearch (separate service), Sonic (Rust), Bleve (Go).

Requirements:
- Embedded in process (no network hop)
- Sub-millisecond query latency at 100M+ docs
- Incremental indexing (near real-time)
- BM25 + custom ranking
- Rust-native

## Decision

**Use Tantivy** as the core search index library.

## Consequences

### Positive
- **Pure Rust** → same process, zero-copy via `mmap`
- **Lucene-inspired architecture** → proven data structures (FST, skip lists, doc values)
- **Fast** → 10M docs, 100 QPS, p99 < 5ms on modest hardware
- **Incremental commits** → new segments visible in < 100ms
- **Flexible schema** → dynamic fields, stored fields, doc values
- **Active development** → Quickwit, Meilisearch, Sonic use it

### Negative
- **No distributed index** → single-node only (mitigated: sharding at application layer)
- **No built-in replication** → handle via shared storage + reader reload
- **Less mature query parser** → custom parser for LYNX syntax
- **Segment merging** can cause latency spikes (mitigated: background merge policy)

## Architecture

```
Indexer (writer)          Search API (readers)
     │                         │
     ▼                         ▼
┌─────────┐               ┌─────────┐
│ Writer  │──segments──▶│ Reader  │ (mmap, no locks)
│ Tantivy │   (NFS)     │ Tantivy │
└─────────┘               └─────────┘
     │                         ▲
     │  Atomic symlink swap    │
     └────── INDEX_RELOAD ─────┘
```

## Alternatives Rejected

| Option | Reason |
| ------ | ------ |
| Lucene + JNI | JVM in-process = GC pauses, memory overhead, complexity |
| Meilisearch | Separate service = network latency, operational burden |
| Sonic | Less features, smaller community |
| Bleve | Go → cross-language boundary |

## Related

- ADR 0004: Index Generation & Atomic Reload
- ADR 0011: Sharding Strategy