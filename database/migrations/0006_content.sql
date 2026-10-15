-- 0006_content.sql
-- +migrate:expand
--
-- Purpose : the document label plane and the link-graph edges.
-- Duration: < 1 s at creation.
-- Plans   : none invalidated.
-- Indexes : created with the tables; no concurrent build.
--
-- The Postgres row is a *label* for a document whose text lives in Tantivy.
-- `doc_id` is the join key in both systems, derived deterministically from the
-- canonical URL (UUID v5) and never reassigned, so a re-crawl updates the same
-- document in place. A `doc_id` mismatch between Postgres and the index is
-- corruption, not a merge case.

CREATE TYPE document_state AS ENUM ('active','shadow_suppressed','soft_404','deleted');

CREATE TABLE IF NOT EXISTS document (
  doc_id             uuid PRIMARY KEY,
  url_id             uuid NOT NULL REFERENCES url(id),
  state              document_state NOT NULL DEFAULT 'active',
  content_hash       bytea,                 -- SipHash of the extracted text
  signature_id       uuid,                  -- dedup_signature / dedup_cluster member
  cluster_id         uuid,

  lang               text,
  published_at       timestamptz,
  modified_at        timestamptz,
  content_type       text,

  -- quality and spam labels, kept for trend analysis and reprocessing
  quality_score      real,
  spam_score         real,
  spam_signals       text[],
  js_required        boolean,

  indexed_generation bigint,
  first_indexed_at   timestamptz,
  last_changed_at    timestamptz NOT NULL DEFAULT now(),
  created_at         timestamptz NOT NULL DEFAULT now(),
  updated_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS document_url_idx     ON document (url_id);
CREATE INDEX IF NOT EXISTS document_cluster_idx ON document (cluster_id) WHERE cluster_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS document_hash_idx    ON document (content_hash);
CREATE INDEX IF NOT EXISTS document_state_idx   ON document (state) WHERE state <> 'active';

-- `document` is current state; `document_version` is history. Only the newest
-- version is indexed. Extracted text is retained here for re-indexing and
-- takedown, not an inverted index.
CREATE TABLE IF NOT EXISTS document_version (
  doc_id         uuid NOT NULL REFERENCES document(doc_id) ON DELETE CASCADE,
  version        integer NOT NULL,          -- monotonic per document
  content_hash   bytea NOT NULL,
  fetched_at     timestamptz NOT NULL DEFAULT now(),
  status_code    integer,
  content_type   text,
  byte_size      integer,
  lang           text,
  extracted_text text,
  PRIMARY KEY (doc_id, version)
);
CREATE INDEX IF NOT EXISTS document_version_fetched_idx ON document_version (fetched_at);

CREATE TABLE IF NOT EXISTS link (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  source_doc_id uuid NOT NULL,              -- logical ref to document(doc_id)
  target_url_id uuid NOT NULL REFERENCES url(id),
  target_doc_id uuid,                       -- null until the target is indexed
  anchor_text   text,
  is_nofollow   boolean NOT NULL DEFAULT false,
  is_sponsored  boolean NOT NULL DEFAULT false,
  context       text,                       -- 'body' | 'nav' | 'footer' | 'main'
  first_seen    timestamptz NOT NULL DEFAULT now(),
  last_seen     timestamptz NOT NULL DEFAULT now(),
  -- Collapses repeated links to the same target, so the graph is an edge set and
  -- not a count: one page with forty links to a host counts once.
  UNIQUE (source_doc_id, target_url_id, is_nofollow)
);
CREATE INDEX IF NOT EXISTS link_target_idx ON link (target_url_id);
CREATE INDEX IF NOT EXISTS link_source_idx ON link (source_doc_id);
CREATE INDEX IF NOT EXISTS link_graph_idx  ON link (target_doc_id) WHERE target_doc_id IS NOT NULL;

-- `raw` is retained because robots.txt is required for compliance and a dispute
-- about what a site asked for must be answerable. It is public configuration
-- text, not content.
CREATE TABLE IF NOT EXISTS robots_policy (
  host_id       uuid PRIMARY KEY REFERENCES host(id) ON DELETE CASCADE,
  raw           text,
  parsed        jsonb NOT NULL,             -- rules, sitemaps, crawl-delay
  status        integer,
  etag          text,
  last_modified text,
  fetched_at    timestamptz NOT NULL DEFAULT now(),
  expires_at    timestamptz NOT NULL,
  parse_error   text
);
CREATE INDEX IF NOT EXISTS robots_policy_expiry_idx ON robots_policy (expires_at);

DROP TRIGGER IF EXISTS document_set_updated_at ON document;
CREATE TRIGGER document_set_updated_at BEFORE UPDATE ON document
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
