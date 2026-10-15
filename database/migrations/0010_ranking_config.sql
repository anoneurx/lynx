-- 0010_ranking_config.sql
-- +migrate:expand
--
-- Purpose : versioned ranking configuration. Every score is reproducible from
--           (doc_id, index generation, ranking_config_version).
-- Duration: < 1 s at creation.
-- Plans   : none invalidated.
-- Indexes : created with the table; no concurrent build.
-- Retention: no_user_data.

CREATE TABLE IF NOT EXISTS ranking_config_version (
  id         uuid PRIMARY KEY DEFAULT uuidv7(),
  version    text NOT NULL UNIQUE,   -- e.g. 'v0.1.0'
  config     jsonb NOT NULL,         -- weights, k1/b, half-lives, penalties
  notes      text,
  created_by uuid,
  created_at timestamptz NOT NULL DEFAULT now(),
  active     boolean NOT NULL DEFAULT false
);
-- Exactly one active configuration; a weight change bumps the version and flips
-- this flag, which is what makes an old score explainable after the fact.
CREATE UNIQUE INDEX IF NOT EXISTS ranking_config_active_uniq
  ON ranking_config_version (active) WHERE active;
