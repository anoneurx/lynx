-- 0007_index_bookkeeping.sql
-- +migrate:expand
--
-- Purpose : index generation manifests, shards, segments, and dedup clusters.
-- Duration: < 1 s at creation.
-- Plans   : none invalidated.
-- Indexes : created with the tables; no concurrent build.
--
-- A generation bump is cheap. Only a `schema_version` or `tokenizer_version`
-- change requires a full rebuild; see ADR-0004. Every score is reproducible from
-- (doc_id, index generation, ranking_config_version).

CREATE TABLE IF NOT EXISTS index_manifest (
  generation             bigint PRIMARY KEY,
  shard_count            integer NOT NULL,
  doc_count              bigint  NOT NULL DEFAULT 0,
  schema_version         text NOT NULL,
  tokenizer_version      text NOT NULL,
  ranking_config_version text NOT NULL,
  path                   text NOT NULL,       -- directory of the sealed generation
  is_current             boolean NOT NULL DEFAULT false,
  built_at               timestamptz NOT NULL DEFAULT now()
);
-- At most one generation is current; the reload signal swaps this atomically.
CREATE UNIQUE INDEX IF NOT EXISTS index_manifest_current_uniq
  ON index_manifest (is_current) WHERE is_current;

CREATE TABLE IF NOT EXISTS index_shard (
  id           uuid PRIMARY KEY DEFAULT uuidv7(),
  generation   bigint NOT NULL REFERENCES index_manifest(generation) ON DELETE CASCADE,
  shard_index  integer NOT NULL,
  doc_count    bigint NOT NULL DEFAULT 0,
  byte_size    bigint NOT NULL DEFAULT 0,
  path         text NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (generation, shard_index)
);

CREATE TABLE IF NOT EXISTS index_segment (
  id         uuid PRIMARY KEY DEFAULT uuidv7(),
  shard_id   uuid NOT NULL REFERENCES index_shard(id) ON DELETE CASCADE,
  segment_id text NOT NULL,
  doc_count  bigint NOT NULL DEFAULT 0,
  byte_size  bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (shard_id, segment_id)
);

-- Two hashes, deliberately: a structural hash ignoring text catches template
-- farms; a text hash catches verbatim copies. One hash for both produces false
-- positives on legitimate templated publishing and false negatives on paraphrased
-- spam. `host_diversity` separates syndication from a doorway network.
CREATE TABLE IF NOT EXISTS dedup_cluster (
  id                    uuid PRIMARY KEY DEFAULT uuidv7(),
  kind                  text NOT NULL,        -- near_duplicate | doorway | template
  confidence            real NOT NULL,
  structural_hash       bytea,                -- DOM shingles, text excluded
  text_hash             bytea,                -- full-text hash
  shingle_count         integer,
  representative_doc_id uuid,
  member_count          integer NOT NULL DEFAULT 0,
  host_diversity        integer NOT NULL DEFAULT 1,
  created_at            timestamptz NOT NULL DEFAULT now(),
  last_seen_at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS dedup_cluster_structural_idx ON dedup_cluster (structural_hash);
CREATE INDEX IF NOT EXISTS dedup_cluster_kind_idx       ON dedup_cluster (kind);
