# Index bookkeeping (`0007_index_bookkeeping.sql`)

Generation bookkeeping for the Tantivy index, which is a separate, derived artifact
(ADR-0002, ADR-0004). The index is rebuilt into `gen-<N>/`, sealed, then made
current by an atomic symlink swap plus an `INDEX_RELOAD` publish.

## `index_manifest`

One row per generation.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `generation` | `bigint` PK | monotonic |
| `shard_count` | `integer` | |
| `doc_count` | `bigint` | |
| `schema_version` | `text` | bumped only for a full-rebuild change |
| `tokenizer_version` | `text` | bumped only for a full-rebuild change |
| `ranking_config_version` | `text` | attribution for reproducibility |
| `path` | `text` | directory of the sealed generation |
| `is_current` | `boolean` | at most one (`index_manifest_current_uniq`) |
| `built_at` | `timestamptz` | |

## `index_shard`

`(generation, shard_index)` UNIQUE; `generation` FK → `index_manifest` with cascade.
Holds `doc_count`, `byte_size`, `path`.

## `index_segment`

`(shard_id, segment_id)` UNIQUE; `shard_id` FK → `index_shard` with cascade. Holds
`doc_count`, `byte_size`. Segment rows are refreshed on every commit.

## `dedup_cluster`

Two hashes because near-duplicate detection needs both: a **structural** hash that
ignores text catches template farms, and a **text** hash catches verbatim copies.
One hash for both produces false positives on legitimate templated publishing and
false negatives on paraphrased spam.

| Column | Type | Notes |
| ------ | ---- | ----- |
| `id` | `uuid` PK | |
| `kind` | `text` | `near_duplicate` \| `doorway` \| `template` |
| `confidence` | `real` | |
| `structural_hash`, `text_hash` | `bytea` | |
| `shingle_count` | `integer` | for banding |
| `representative_doc_id` | `uuid` | the member shown |
| `member_count` | `integer` | |
| `host_diversity` | `integer` | separates syndication from a doorway network |

`host_diversity` is the number that distinguishes a syndicated press release (many
hosts, one legitimate document) from a doorway network (many hosts, one template).
Both look identical structurally.
