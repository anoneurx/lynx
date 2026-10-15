-- 0009_search_quality.sql
-- +migrate:expand
--
-- Purpose : the ranking evaluation harness -- runs and judgements.
-- Duration: < 1 s at creation.
-- Plans   : none invalidated.
-- Indexes : created with the tables; no concurrent build.
-- Retention: aggregate.
--
-- Judgements are the reviewed golden set that gates ranking changes. A judgement
-- stores only an opaque `query_id` into the versioned evaluation corpus, never a
-- query string: there is no column here in which a user query could be recorded.

CREATE TABLE IF NOT EXISTS quality_evaluation_run (
  id                     uuid PRIMARY KEY DEFAULT uuidv7(),
  name                   text NOT NULL,
  index_generation       bigint,
  ranking_config_version text,
  query_set              text NOT NULL,
  ndcg_at_10             real,
  precision_at_10        real,
  recall                 real,
  mrr                    real,
  duplicate_rate         real,
  spam_rate              real,
  passed                 boolean,
  notes                  text,
  started_at             timestamptz NOT NULL DEFAULT now(),
  finished_at            timestamptz
);

CREATE TABLE IF NOT EXISTS relevance_judgement (
  id         uuid PRIMARY KEY DEFAULT uuidv7(),
  run_id     uuid REFERENCES quality_evaluation_run(id) ON DELETE SET NULL,
  query_id   text NOT NULL,      -- opaque id into the versioned query corpus
  doc_id     uuid NOT NULL,
  grade      smallint NOT NULL,  -- 0..4
  labeler    text NOT NULL,      -- rule | human | model
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (grade BETWEEN 0 AND 4)
);
CREATE INDEX IF NOT EXISTS relevance_judgement_query_idx ON relevance_judgement (query_id);
CREATE INDEX IF NOT EXISTS relevance_judgement_run_idx   ON relevance_judgement (run_id);
